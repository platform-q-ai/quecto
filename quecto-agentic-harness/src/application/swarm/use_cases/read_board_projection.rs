//! The board projection a supervising client reads on its parent's socket
//! (#2475, #2478): the run, every item, the members and the totals, in one
//! read transaction. Rust-only: Python's board had no such read.
use std::sync::Arc;

use serde_json::Value;

use super::OverRepository;
use super::create_task::TASK_BOARD_CAPACITY;
use crate::application::swarm::board_control::edited;
use crate::application::swarm::board_operation::atomic;
use crate::application::swarm::board_read_models::counts;
use crate::application::swarm::board_tasks::{HELD_CLAIM, read_task};
use crate::application::swarm::dto::{
    BoardProjection, DictRow, EvidenceProjection, FileRow, ItemProjection, MemberProjection,
    MemberRole, RunProjection, TaskRow, UsageStanding,
};
use crate::application::swarm::ports::{BoardRepository, BoardTransaction};
use crate::domain::swarm::BoardError;

/// Membership-free, like `_status`: the supervising session reads it from
/// outside the run, whose members may all be gone. One plain transaction,
/// as `_status` and the control receipt read: the usage ledger's tables
/// are ensured on read, which a `query_only` read transaction refuses, so
/// a client that only wants to know whether the board moved polls the
/// revision (`_event_cursor`) instead.
pub struct ReadBoardProjection {
    repository: Arc<dyn BoardRepository>,
}

impl ReadBoardProjection {
    pub fn new(repository: Arc<dyn BoardRepository>) -> Self {
        Self { repository }
    }

    /// # Errors
    /// An edited record, or the store's refusal (a missing board,
    /// contention).
    pub fn execute(&self) -> Result<BoardProjection, BoardError> {
        atomic(&*self.repository, false, project)
    }
}

impl OverRepository for ReadBoardProjection {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self { repository }
    }
}

fn project(transaction: &dyn BoardTransaction) -> Result<BoardProjection, BoardError> {
    let _ = (
        item as fn(&TaskRow, &[FileRow]) -> ItemProjection,
        evidence as fn(&DictRow) -> EvidenceProjection,
    );
    let _ = (
        members,
        budget as fn(UsageStanding) -> _,
        TASK_BOARD_CAPACITY,
        HELD_CLAIM,
    );
    let _: Option<RunProjection> = None;
    let _ = read_task(transaction, &Value::Null);
    Ok(BoardProjection {
        revision: transaction.event_generation()?,
        run: None,
        items: Vec::new(),
        members: Vec::new(),
        evidence: Vec::new(),
        totals: counts(transaction, None)?,
    })
}

fn item(task: &TaskRow, files: &[FileRow]) -> ItemProjection {
    let column = |name: &str| task.get(name).cloned().unwrap_or(Value::Null);
    let id = column("id");
    let reserved_paths = files
        .iter()
        .filter(|file| file_column(file, "task") == Some(&id))
        .filter_map(|file| file_column(file, "path").cloned())
        .collect();
    ItemProjection {
        title: column("title"),
        criteria: column("acceptance"),
        depends_on: column("dependencies"),
        state: column("status"),
        owner: column("owner"),
        blocker: column("blocker"),
        evidence: column("evidence"),
        reserved_paths,
        id,
    }
}

fn file_column<'a>(file: &'a FileRow, name: &str) -> Option<&'a Value> {
    file.columns
        .iter()
        .find(|(column, _)| column == name)
        .map(|(_, value)| value)
}

fn members(
    transaction: &dyn BoardTransaction,
    tasks: &[TaskRow],
    coordinator: &Value,
    integrator: &Value,
) -> Result<Vec<MemberProjection>, BoardError> {
    let rows = transaction.members()?;
    let names: Vec<&str> = rows.iter().filter_map(|row| row.text("id")).collect();
    let latest = transaction.latest_activity(&names)?;
    Ok(rows
        .iter()
        .map(|row| {
            let id = row.get("id").cloned().unwrap_or(Value::Null);
            let role = match &id {
                id if id == coordinator => MemberRole::Coordinator,
                id if id == integrator => MemberRole::Integrator,
                _ => MemberRole::Worker,
            };
            let claims = tasks
                .iter()
                .filter(|task| task.get("owner") == Some(&id))
                .filter(|task| {
                    task.text("status")
                        .is_some_and(|status| HELD_CLAIM.contains(&status))
                })
                .filter_map(|task| task.get("id").cloned())
                .collect();
            let last_activity = latest
                .iter()
                .find(|activity| Some(activity.actor.as_str()) == id.as_str())
                .map_or(Value::Null, |activity| activity.latest.clone());
            MemberProjection {
                role,
                status: row.get("status").cloned().unwrap_or(Value::Null),
                claims,
                last_activity,
                id,
            }
        })
        .collect())
}

fn evidence(row: &DictRow) -> EvidenceProjection {
    let column = |name: &str| row.get(name).cloned().unwrap_or(Value::Null);
    EvidenceProjection {
        criterion: column("criterion"),
        artifact: column("artifact"),
        revision: column("revision"),
        kind: column("kind"),
        actor: column("actor"),
        accepted: column("accepted"),
    }
}

/// The budget as stored, then its ledger's totals, as the control receipt
/// reads them.
fn budget(standing: UsageStanding) -> Result<serde_json::Map<String, Value>, BoardError> {
    let Value::Object(mut budget) = standing.budget else {
        return Err(edited("usage budget"));
    };
    budget.insert("observed_tokens".to_owned(), standing.observed_tokens);
    budget.insert(
        "unknown_usage_requests".to_owned(),
        standing.unknown_usage_requests,
    );
    Ok(budget)
}

#[cfg(test)]
#[path = "read_board_projection_tests.rs"]
mod tests;
