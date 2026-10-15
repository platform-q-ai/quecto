//! The board projection a supervising client reads on its parent's socket
//! (#2475, #2478): the run, its items, members, evidence and totals, read
//! together through the board's projection reads. Rust-only: Python's
//! board had no such read.
//!
//! The rows are copied in one read transaction and typed after it ends
//! (#2487 round 2). What a projection carries is bounded twice: the store
//! copies no row past [`ProjectionLimits::bytes`] of stored text, and the
//! rows kept are counted as the socket line encodes them (a control
//! character is six bytes there), within the same budget.
use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::Value;

use super::OverRepository;
use super::create_task::TASK_BOARD_CAPACITY;
use crate::application::swarm::board_control::edited;
use crate::application::swarm::board_operation::read;
use crate::application::swarm::dto::{
    BoardProjection, BoardRevision, BudgetProjection, EvidenceProjection, ItemProjection,
    MemberClaimCounts, MemberProjection, MemberRole, ProjectionLimits, ProjectionRows,
    ProjectionTotals, RunProjection, StoredEvidenceColumns, StoredItemColumns, StoredMemberColumns,
    StoredRunColumns, StoredStateColumns,
};
use crate::application::swarm::ports::BoardRepository;
use crate::domain::swarm::telemetry::run_role;
use crate::domain::swarm::{
    BoardError, BoardRole, MemberState, RefusalKind, RunState, TaskState, WORK_HOLDING_STATUSES,
    participates,
};

/// The most bytes of rows one projection carries, well inside a socket
/// line (8 MiB): rows past it are left out and the projection says so.
pub const PROJECTION_BUDGET_BYTES: usize = 4 * 1024 * 1024;

/// The rows a projection reads at most: every item a board may hold, and
/// far more members, reservations and evidence than a run makes; and the
/// stored text it copies at most, the budget.
pub const PROJECTION_LIMITS: ProjectionLimits = ProjectionLimits {
    items: TASK_BOARD_CAPACITY,
    files: 2_000,
    members: 100,
    evidence: 500,
    bytes: PROJECTION_BUDGET_BYTES as i64,
};

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

    /// [`Self::execute`] within `budget` bytes of rows, as stored and as
    /// encoded.
    ///
    /// # Errors
    /// As [`Self::execute`].
    pub fn within(&self, budget: usize) -> Result<BoardProjection, BoardError> {
        let limits = ProjectionLimits {
            bytes: i64::try_from(budget).unwrap_or(i64::MAX),
            ..PROJECTION_LIMITS
        };
        let (rows, claim_counts) = read(&*self.repository, |transaction| {
            let rows = transaction.projection_rows(limits)?;
            let coordinator = rows
                .run
                .as_ref()
                .and_then(|run| run.coordinator.as_str().map(str::to_owned));
            let claim_counts = transaction.claim_counts(coordinator.as_deref())?;
            Ok((rows, claim_counts))
        })?;
        project(rows, claim_counts, budget)
    }

    /// The board's revision and the run it counts: what a client polls to
    /// learn whether the board moved, or was made again.
    ///
    /// # Errors
    /// The store's refusal; a run id that is not text.
    pub fn revision(&self) -> Result<BoardRevision, BoardError> {
        let stored = read(&*self.repository, |transaction| {
            transaction.projection_revision()
        })?;
        let created = match &stored.deadline {
            Value::Null => false,
            deadline => participates(deadline.as_f64().ok_or_else(|| edited("run deadline"))?),
        };
        Ok(BoardRevision {
            run: optional_text("run", &stored.run)?,
            revision: stored.revision,
            created,
        })
    }
}

impl OverRepository for ReadBoardProjection {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self { repository }
    }
}

/// Each item's state, dependencies and owner, by id.
type States = BTreeMap<i64, (TaskState, Vec<i64>, Option<String>)>;

fn project(
    rows: ProjectionRows,
    claim_counts: MemberClaimCounts,
    budget: usize,
) -> Result<BoardProjection, BoardError> {
    // The board refuses to create an item past its capacity: a board
    // holding more was edited, and its states were not all read.
    if rows.item_count > TASK_BOARD_CAPACITY || rows.states.len() as i64 != rows.item_count {
        return Err(edited("tasks"));
    }
    let run = rows
        .run
        .as_ref()
        .map(|run| typed_run(run, &rows))
        .transpose()?;
    let mut spent = 0;
    if let Some(run) = &run
        && !fits(&mut spent, budget, run_bytes(run))
    {
        return Err(edited("run"));
    }
    let states = typed_states(&rows.states)?;
    let mut totals = item_totals(&states)?;
    let coordinator = run.as_ref().and_then(|run| run.coordinator.clone());
    let integrator = run.as_ref().and_then(|run| run.integrator.clone());
    let mut truncated = rows.item_count > rows.items.len() as i64
        || rows.file_count > rows.files.len() as i64
        || rows.member_count > rows.members.len() as i64
        || rows.evidence_count > rows.evidence.len() as i64;
    let paths = reserved_paths(&rows.files)?;
    let mut items = Vec::new();
    for stored in &rows.items {
        let item = typed_item(stored, &states, &paths)?;
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
            &states,
        )?;
        if !fits(&mut spent, budget, member_bytes(&member)) {
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
    debug_assert!(spent <= budget, "the rows kept fit the budget");
    totals.members = rows.member_count;
    totals.members_without_claim = claim_counts.members_without_claim;
    totals.members_dead = claim_counts.members_dead;
    Ok(BoardProjection {
        revision: rows.revision,
        run,
        items,
        members,
        evidence,
        totals,
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

/// What a row's keys, punctuation, flags and numbers come to at most, as
/// the projection's answer encodes it: an item, a member, an evidence row,
/// and the run with its budget.
const ITEM_FRAME: usize = 160;
const MEMBER_FRAME: usize = 96;
const EVIDENCE_FRAME: usize = 112;
const RUN_FRAME: usize = 512;
/// The most a number takes encoded: an `i64`'s 20 characters, or an
/// `f64`'s 24.
const NUMBER: usize = 24;

/// `text` as the answer's JSON encodes it, quotes and escapes included:
/// a quote, a backslash and the five control characters with a short
/// escape take two bytes, every other control character six (`\u0001`),
/// and the rest their UTF-8 bytes.
fn encoded_text(text: &str) -> usize {
    text.chars().fold(2, |total, character| {
        let size = match character {
            '"' | '\\' | '\u{8}' | '\u{c}' | '\n' | '\r' | '\t' => 2,
            '\u{0}'..='\u{1f}' => 6,
            other => other.len_utf8(),
        };
        total.saturating_add(size)
    })
}

/// `value` as the answer's JSON encodes it.
fn encoded_value(value: &Value) -> usize {
    match value {
        Value::Null | Value::Bool(true) => 4,
        Value::Bool(false) => 5,
        Value::Number(number) => number.to_string().len(),
        Value::String(text) => encoded_text(text),
        Value::Array(entries) => encoded_list(entries.iter().map(encoded_value)),
        Value::Object(entries) => encoded_list(
            entries
                .iter()
                .map(|(key, value)| encoded_text(key).saturating_add(1 + encoded_value(value))),
        ),
    }
}

fn encoded_optional(text: Option<&String>) -> usize {
    text.map_or(4, |text| encoded_text(text))
}

/// A list (or object) of entries each `sizes` long, with its brackets and
/// the commas between its entries.
fn encoded_list(sizes: impl Iterator<Item = usize>) -> usize {
    let (count, total) = sizes.fold((0_usize, 2_usize), |(count, total), size| {
        (count + 1, total.saturating_add(size))
    });
    total.saturating_add(count.saturating_sub(1))
}

fn run_bytes(run: &RunProjection) -> usize {
    [
        RUN_FRAME,
        encoded_text(&run.id),
        encoded_text(&run.goal),
        encoded_text(run.status.as_str()),
        encoded_optional(run.outcome.as_ref()),
        encoded_optional(run.outcome_reason.as_ref()),
        encoded_value(&run.criteria),
        encoded_optional(run.coordinator.as_ref()),
        encoded_optional(run.integrator.as_ref()),
    ]
    .into_iter()
    .fold(0, usize::saturating_add)
}

fn item_bytes(item: &ItemProjection) -> usize {
    [
        ITEM_FRAME,
        NUMBER,
        encoded_text(&item.title),
        encoded_list(
            item.criteria
                .iter()
                .map(|criterion| encoded_text(criterion)),
        ),
        encoded_list(item.depends_on.iter().map(|_| NUMBER)),
        encoded_text(item.state.as_str()),
        encoded_optional(item.owner.as_ref()),
        encoded_optional(item.blocker.as_ref()),
        encoded_list(item.reserved_paths.iter().map(|path| encoded_text(path))),
        encoded_value(&item.evidence),
    ]
    .into_iter()
    .fold(0, usize::saturating_add)
}

fn member_bytes(member: &MemberProjection) -> usize {
    [
        MEMBER_FRAME,
        encoded_text(&member.id),
        encoded_text(member.role.as_str()),
        encoded_text(member.status.as_str()),
        encoded_list(member.claims.iter().map(|_| NUMBER)),
        NUMBER,
    ]
    .into_iter()
    .fold(0, usize::saturating_add)
}

fn evidence_bytes(row: &EvidenceProjection) -> usize {
    [
        EVIDENCE_FRAME,
        encoded_text(&row.criterion),
        encoded_optional(row.artifact.as_ref()),
        encoded_optional(row.revision.as_ref()),
        encoded_optional(row.kind.as_ref()),
        encoded_text(&row.actor),
    ]
    .into_iter()
    .fold(0, usize::saturating_add)
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
        status: run
            .status
            .as_str()
            .and_then(RunState::known)
            .ok_or_else(|| edited("run status"))?,
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

/// The budget's three settings as a board stored them (none set: no
/// limit, not strict, not warned) and the ledger's two totals. No budget
/// is applied (owner decision, 2026-10-14), so each setting is reported
/// when it reads as the board wrote it (a limit null or a whole number of
/// tokens, two flags) and as unset when it does not, never refusing the
/// projection (review round 1, #2569).
fn budget(rows: &ProjectionRows) -> Result<BudgetProjection, BoardError> {
    let stored = match &rows.budget {
        Some(Value::Object(stored)) => Some(stored),
        Some(_) | None => None,
    };
    let setting = |name: &str| stored.and_then(|stored| stored.get(name));
    let flag = |name: &str| matches!(setting(name), Some(Value::Bool(true)));
    let token_limit = setting("token_limit")
        .and_then(Value::as_u64)
        .and_then(|limit| i64::try_from(limit).ok());
    let (strict_unknown, warned) = (flag("strict_unknown"), flag("warned"));
    Ok(BudgetProjection {
        token_limit,
        strict_unknown,
        warned,
        observed_tokens: integer("usage ledger", &rows.observed_tokens)?,
        unknown_usage_requests: integer("usage ledger", &rows.unknown_usage_requests)?,
    })
}

/// Every item's state, dependencies and owner, typed.
fn typed_states(stored: &[StoredStateColumns]) -> Result<States, BoardError> {
    let mut states = BTreeMap::new();
    for row in stored {
        states.insert(
            integer("task", &row.id)?,
            (
                state("task status", &row.status)?,
                ids("task dependencies", &row.depends_on)?,
                optional_text("task owner", &row.owner)?,
            ),
        );
    }
    Ok(states)
}

/// The totals over every item: `waiting` counts the ready items one of
/// whose dependencies is not completed.
fn item_totals(states: &States) -> Result<ProjectionTotals, BoardError> {
    let mut totals = ProjectionTotals {
        items: states.len() as i64,
        ..ProjectionTotals::default()
    };
    for (state, depends_on, _) in states.values() {
        let count = match state.as_str() {
            "ready" => &mut totals.ready,
            "claimed" => &mut totals.claimed,
            "blocked" => &mut totals.blocked,
            "submitted" => &mut totals.submitted,
            "completed" => &mut totals.completed,
            other => {
                return Err(BoardError::new(
                    RefusalKind::Internal,
                    format!("an item state {other:?} passed the known states"),
                ));
            }
        };
        *count += 1;
        if *state == TaskState::READY && waits(depends_on, states) {
            totals.waiting += 1;
        }
    }
    Ok(totals)
}

/// One of the five statuses the board writes; anything else is edited.
fn state(record: &str, value: &Value) -> Result<TaskState, BoardError> {
    value
        .as_str()
        .and_then(TaskState::known)
        .ok_or_else(|| edited(record))
}

fn ids(record: &str, value: &Value) -> Result<Vec<i64>, BoardError> {
    match value {
        Value::Array(ids) => ids.iter().map(|id| integer(record, id)).collect(),
        _ => Err(edited(record)),
    }
}

/// One dependency is not completed (or names no item).
fn waits(depends_on: &[i64], states: &States) -> bool {
    depends_on.iter().any(|id| {
        states
            .get(id)
            .is_none_or(|(state, _, _)| *state != TaskState::COMPLETED)
    })
}

/// The owner's claim is held in `state` (claimed, blocked or submitted).
fn holds_claim(state: &TaskState) -> bool {
    WORK_HOLDING_STATUSES.contains(&state.as_str())
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
    states: &States,
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
        waiting: state == TaskState::READY && waits(&depends_on, states),
        depends_on,
        state,
        owner: optional_text("task owner", &stored.owner)?,
        blocker: optional_text("task blocker", &stored.blocker)?,
        evidence: stored.evidence.clone(),
        reserved_paths: paths.get(&id).cloned().unwrap_or_default(),
    })
}

/// A listed member's role: one of the three a run gives its members; any
/// other answer is the harness's own error, never a guessed role.
fn member_role(role: Option<BoardRole>) -> Result<MemberRole, BoardError> {
    match role {
        Some(BoardRole::Coordinator) => Ok(MemberRole::Coordinator),
        Some(BoardRole::Integrator) => Ok(MemberRole::Integrator),
        Some(BoardRole::Worker) => Ok(MemberRole::Worker),
        Some(BoardRole::Host) | None => Err(BoardError::new(
            RefusalKind::Internal,
            "a listed member was given no run role",
        )),
    }
}

/// A member, whose claims are read from every item the board holds.
fn typed_member(
    stored: &StoredMemberColumns,
    coordinator: Option<&str>,
    integrator: Option<&str>,
    states: &States,
) -> Result<MemberProjection, BoardError> {
    let id = text("member", &stored.id)?;
    let role = member_role(run_role(&id, coordinator, integrator, true))?;
    let claims = states
        .iter()
        .filter(|(_, (state, _, owner))| {
            owner.as_deref() == Some(id.as_str()) && holds_claim(state)
        })
        .map(|(item, _)| *item)
        .collect();
    let last_activity = match &stored.last_activity {
        Value::Null => None,
        time => Some(time.as_f64().ok_or_else(|| edited("member event time"))?),
    };
    Ok(MemberProjection {
        status: stored
            .status
            .as_str()
            .and_then(MemberState::known)
            .ok_or_else(|| edited("member status"))?,
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
