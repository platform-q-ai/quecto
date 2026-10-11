//! The board projection a supervising client reads on its parent's socket
//! (#2475, #2478): the run, its items, members, evidence and totals, read
//! together through the board's projection reads. Rust-only: Python's
//! board had no such read.
use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::Value;

use super::OverRepository;
use super::create_task::TASK_BOARD_CAPACITY;
use crate::application::swarm::board_control::edited;
use crate::application::swarm::board_operation::read;
use crate::application::swarm::dto::{
    BoardProjection, BudgetProjection, CountedTask, EvidenceProjection, ItemProjection, ItemState,
    MemberProjection, MemberRole, ProjectionLimits, ProjectionRows, ProjectionTotals,
    RunProjection, StoredEvidenceColumns, StoredItemColumns, StoredMemberColumns, StoredRunColumns,
};
use crate::application::swarm::ports::{BoardRepository, BoardTransaction};
use crate::domain::swarm::telemetry::run_role;
use crate::domain::swarm::{BoardError, BoardRole};

/// The rows a projection reads at most: every item a board may hold, and
/// far more members, reservations and evidence than a run makes.
pub const PROJECTION_LIMITS: ProjectionLimits = ProjectionLimits {
    items: TASK_BOARD_CAPACITY,
    files: 2_000,
    members: 100,
    evidence: 500,
};

/// The most the rows of one projection may hold, well inside a socket
/// line (8 MiB): rows past it are left out and the projection says so.
pub const PROJECTION_BUDGET_BYTES: usize = 4 * 1024 * 1024;

/// Membership-free, like `_status`: the supervising session reads it from
/// outside the run, whose members may all be gone. One read transaction
/// (#2338) of plain SELECTs: it takes no write lock, creates and alters
/// nothing, and so never holds a member up or migrates an older board.
pub struct ReadBoardProjection {
    repository: Arc<dyn BoardRepository>,
}

impl ReadBoardProjection {
    pub fn new(repository: Arc<dyn BoardRepository>) -> Self {
        Self { repository }
    }

    /// # Errors
    /// A value the board never writes (an edited record), or the store's
    /// refusal (a missing board, contention).
    pub fn execute(&self) -> Result<BoardProjection, BoardError> {
        self.within(PROJECTION_BUDGET_BYTES)
    }

    /// [`Self::execute`] within `budget` bytes of rows.
    ///
    /// # Errors
    /// As [`Self::execute`].
    pub fn within(&self, budget: usize) -> Result<BoardProjection, BoardError> {
        read(&*self.repository, |transaction| {
            project(transaction, budget)
        })
    }

    /// The board's revision alone: what a client polls to learn whether
    /// the board moved.
    ///
    /// # Errors
    /// The store's refusal.
    pub fn revision(&self) -> Result<i64, BoardError> {
        read(&*self.repository, |transaction| {
            transaction.projection_revision()
        })
    }
}

impl OverRepository for ReadBoardProjection {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self { repository }
    }
}

fn project(
    transaction: &dyn BoardTransaction,
    budget: usize,
) -> Result<BoardProjection, BoardError> {
    let rows = transaction.projection_rows(PROJECTION_LIMITS)?;
    let states = transaction.task_states()?;
    let run = rows
        .run
        .as_ref()
        .map(|run| typed_run(run, &rows))
        .transpose()?;
    let coordinator = run.as_ref().and_then(|run| run.coordinator.clone());
    let integrator = run.as_ref().and_then(|run| run.integrator.clone());
    let claim_counts = transaction.claim_counts(coordinator.as_deref())?;
    let (status_of, totals) = item_totals(&states)?;
    let mut spent = run
        .as_ref()
        .map_or(0, |run| run.criteria.to_string().len() + run.goal.len());
    let mut truncated = rows.item_count > rows.items.len() as i64
        || rows.file_count > rows.files.len() as i64
        || rows.member_count > rows.members.len() as i64
        || rows.evidence_count > rows.evidence.len() as i64;
    let paths = reserved_paths(&rows.files)?;
    let mut items = Vec::new();
    for stored in &rows.items {
        let item = typed_item(stored, &status_of, &paths)?;
        if !fits(&mut spent, budget, item_bytes(&item)) {
            truncated = true;
            break;
        }
        items.push(item);
    }
    let mut members = Vec::new();
    for stored in &rows.members {
        let member = typed_member(
            stored,
            coordinator.as_deref(),
            integrator.as_deref(),
            &items,
        )?;
        if !fits(
            &mut spent,
            budget,
            member.id.len() + member.status.len() + 64,
        ) {
            truncated = true;
            break;
        }
        members.push(member);
    }
    let mut evidence = Vec::new();
    for stored in &rows.evidence {
        let row = typed_evidence(stored)?;
        if !fits(&mut spent, budget, evidence_bytes(&row)) {
            truncated = true;
            break;
        }
        evidence.push(row);
    }
    Ok(BoardProjection {
        revision: rows.revision,
        run,
        items,
        members,
        evidence,
        totals: ProjectionTotals {
            members: rows.member_count,
            members_without_claim: claim_counts.members_without_claim,
            members_dead: claim_counts.members_dead,
            ..totals
        },
        truncated,
    })
}

/// Adds `bytes` to what was spent when it stays within `budget`.
fn fits(spent: &mut usize, budget: usize, bytes: usize) -> bool {
    match spent.checked_add(bytes) {
        Some(total) if total <= budget => {
            *spent = total;
            true
        }
        _ => false,
    }
}

fn item_bytes(item: &ItemProjection) -> usize {
    let texts = item.title.len()
        + item.owner.as_ref().map_or(0, String::len)
        + item.blocker.as_ref().map_or(0, String::len);
    let lists = item.criteria.iter().map(String::len).sum::<usize>()
        + item.reserved_paths.iter().map(String::len).sum::<usize>()
        + item.depends_on.len() * 21;
    texts + lists + item.evidence.to_string().len() + 128
}

fn evidence_bytes(row: &EvidenceProjection) -> usize {
    let optional = |text: &Option<String>| text.as_ref().map_or(0, String::len);
    row.criterion.len()
        + row.actor.len()
        + optional(&row.artifact)
        + optional(&row.revision)
        + optional(&row.kind)
        + 64
}

fn text(record: &str, value: &Value) -> Result<String, BoardError> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| edited(record))
}

fn optional_text(record: &str, value: &Value) -> Result<Option<String>, BoardError> {
    match value {
        Value::Null => Ok(None),
        Value::String(text) => Ok(Some(text.clone())),
        _ => Err(edited(record)),
    }
}

fn integer(record: &str, value: &Value) -> Result<i64, BoardError> {
    value.as_i64().ok_or_else(|| edited(record))
}

fn typed_run(run: &StoredRunColumns, rows: &ProjectionRows) -> Result<RunProjection, BoardError> {
    let criteria = match &run.criteria {
        criteria @ Value::Array(_) => criteria.clone(),
        _ => return Err(edited("run criteria")),
    };
    Ok(RunProjection {
        id: text("run", &run.id)?,
        goal: text("run goal", &run.goal)?,
        status: text("run status", &run.status)?,
        outcome: optional_text("run outcome", &run.outcome)?,
        outcome_reason: optional_text("run outcome", &run.outcome_reason)?,
        generation: rows.generation,
        criteria,
        member_limit: integer("run member limit", &run.member_limit)?,
        deadline: run
            .deadline
            .as_f64()
            .ok_or_else(|| edited("run deadline"))?,
        coordinator: optional_text("run coordinator", &run.coordinator)?,
        integrator: optional_text("run integrator", &run.integrator)?,
        budget: budget(rows)?,
    })
}

/// The budget's three settings as the board writes them (none set: no
/// limit, not strict, not warned) and the ledger's two totals.
fn budget(rows: &ProjectionRows) -> Result<BudgetProjection, BoardError> {
    let (token_limit, strict_unknown, warned) = match &rows.budget {
        None => (None, false, false),
        Some(Value::Object(stored)) => {
            let flag = |name: &str| match stored.get(name) {
                None => Ok(false),
                Some(Value::Bool(flag)) => Ok(*flag),
                Some(_) => Err(edited("usage budget")),
            };
            let limit = match stored.get("token_limit") {
                None | Some(Value::Null) => None,
                Some(limit) => Some(limit.as_i64().ok_or_else(|| edited("usage budget"))?),
            };
            (limit, flag("strict_unknown")?, flag("warned")?)
        }
        Some(_) => return Err(edited("usage budget")),
    };
    Ok(BudgetProjection {
        token_limit,
        strict_unknown,
        warned,
        observed_tokens: integer("usage ledger", &rows.observed_tokens)?,
        unknown_usage_requests: integer("usage ledger", &rows.unknown_usage_requests)?,
    })
}

/// Every item's state, and the totals over them: `waiting` counts the
/// ready items one of whose dependencies is not completed.
fn item_totals(
    states: &[CountedTask],
) -> Result<(BTreeMap<i64, ItemState>, ProjectionTotals), BoardError> {
    let mut status_of = BTreeMap::new();
    for task in states {
        status_of.insert(
            integer("task", &task.id)?,
            state("task status", &task.status)?,
        );
    }
    let mut totals = ProjectionTotals {
        items: states.len() as i64,
        ..ProjectionTotals::default()
    };
    for task in states {
        let state = status_of[&integer("task", &task.id)?];
        let count = match state {
            ItemState::Ready => &mut totals.ready,
            ItemState::Claimed => &mut totals.claimed,
            ItemState::Blocked => &mut totals.blocked,
            ItemState::Submitted => &mut totals.submitted,
            ItemState::Completed => &mut totals.completed,
        };
        *count += 1;
        if state == ItemState::Ready
            && waits(&ids("task dependencies", &task.dependencies)?, &status_of)
        {
            totals.waiting += 1;
        }
    }
    Ok((status_of, totals))
}

fn state(record: &str, value: &Value) -> Result<ItemState, BoardError> {
    value
        .as_str()
        .and_then(ItemState::parse)
        .ok_or_else(|| edited(record))
}

fn ids(record: &str, value: &Value) -> Result<Vec<i64>, BoardError> {
    match value {
        Value::Array(ids) => ids.iter().map(|id| integer(record, id)).collect(),
        _ => Err(edited(record)),
    }
}

/// One dependency is not completed (or names no item).
fn waits(depends_on: &[i64], status_of: &BTreeMap<i64, ItemState>) -> bool {
    depends_on
        .iter()
        .any(|id| status_of.get(id) != Some(&ItemState::Completed))
}

fn reserved_paths(files: &[(Value, Value)]) -> Result<BTreeMap<i64, Vec<String>>, BoardError> {
    let mut paths: BTreeMap<i64, Vec<String>> = BTreeMap::new();
    for (path, task) in files {
        let path = text("file reservation", path)?;
        paths
            .entry(integer("file reservation", task)?)
            .or_default()
            .push(path);
    }
    Ok(paths)
}

fn typed_item(
    stored: &StoredItemColumns,
    status_of: &BTreeMap<i64, ItemState>,
    paths: &BTreeMap<i64, Vec<String>>,
) -> Result<ItemProjection, BoardError> {
    let id = integer("task", &stored.id)?;
    let state = state("task status", &stored.status)?;
    let depends_on = ids("task dependencies", &stored.depends_on)?;
    let criteria = match &stored.criteria {
        Value::Array(criteria) => criteria
            .iter()
            .map(|criterion| text("task acceptance", criterion))
            .collect(),
        _ => Err(edited("task acceptance")),
    }?;
    Ok(ItemProjection {
        id,
        title: text("task title", &stored.title)?,
        criteria,
        waiting: state == ItemState::Ready && waits(&depends_on, status_of),
        depends_on,
        state,
        owner: optional_text("task owner", &stored.owner)?,
        blocker: optional_text("task blocker", &stored.blocker)?,
        evidence: stored.evidence.clone(),
        reserved_paths: paths.get(&id).cloned().unwrap_or_default(),
    })
}

fn typed_member(
    stored: &StoredMemberColumns,
    coordinator: Option<&str>,
    integrator: Option<&str>,
    items: &[ItemProjection],
) -> Result<MemberProjection, BoardError> {
    let id = text("member", &stored.id)?;
    let role = match run_role(&id, coordinator, integrator, true) {
        Some(BoardRole::Coordinator) => MemberRole::Coordinator,
        Some(BoardRole::Integrator) => MemberRole::Integrator,
        Some(BoardRole::Worker | BoardRole::Host) | None => MemberRole::Worker,
    };
    let claims = items
        .iter()
        .filter(|item| item.owner.as_deref() == Some(id.as_str()) && item.state.holds_claim())
        .map(|item| item.id)
        .collect();
    let last_activity = match &stored.last_activity {
        Value::Null => None,
        time => Some(time.as_f64().ok_or_else(|| edited("member event time"))?),
    };
    Ok(MemberProjection {
        status: text("member status", &stored.status)?,
        role,
        claims,
        last_activity,
        id,
    })
}

fn typed_evidence(stored: &StoredEvidenceColumns) -> Result<EvidenceProjection, BoardError> {
    let accepted = match &stored.accepted {
        Value::Bool(accepted) => *accepted,
        Value::Number(number) if number.as_i64() == Some(1) => true,
        Value::Number(number) if number.as_i64() == Some(0) => false,
        _ => return Err(edited("evidence")),
    };
    Ok(EvidenceProjection {
        criterion: text("evidence", &stored.criterion)?,
        artifact: optional_text("evidence", &stored.artifact)?,
        revision: optional_text("evidence", &stored.revision)?,
        kind: optional_text("evidence", &stored.kind)?,
        actor: text("evidence", &stored.actor)?,
        accepted,
    })
}

#[cfg(test)]
#[path = "read_board_projection_tests.rs"]
mod tests;
