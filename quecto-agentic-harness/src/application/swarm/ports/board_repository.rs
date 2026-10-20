//! The coordination board's transaction boundary: every role over one
//! transaction, the repository that opens it, and the ids and JSON encoding it
//! writes.

use super::board_events::BoardEvents;
use super::board_evidence::BoardEvidence;
use super::board_files::BoardFiles;
use super::board_members::BoardMembers;
use super::board_messages::BoardMessages;
use super::board_projection::BoardProjectionReads;
use super::board_requests::BoardRequests;
use super::board_runs::BoardRuns;
use super::board_tasks::BoardTasks;
use super::board_usage::BoardUsage;
use super::board_wakes::BoardWakes;
use crate::domain::swarm::value_objects::board_error::BoardError;
use serde_json::Value;

/// Every role over one transaction.
pub trait BoardTransaction:
    BoardRuns
    + BoardMembers
    + BoardEvents
    + BoardTasks
    + BoardRequests
    + BoardFiles
    + BoardMessages
    + BoardWakes
    + BoardUsage
    + BoardEvidence
    + BoardProjectionReads
{
}

impl<T> BoardTransaction for T where
    T: BoardRuns
        + BoardMembers
        + BoardEvents
        + BoardTasks
        + BoardRequests
        + BoardFiles
        + BoardMessages
        + BoardWakes
        + BoardUsage
        + BoardEvidence
        + BoardProjectionReads
        + ?Sized
{
}

/// The work one board transaction runs.
pub type BoardWork<'w> = dyn FnMut(&dyn BoardTransaction) -> Result<(), BoardError> + 'w;

/// The board's transaction boundary (`Store.transaction(create)`): `work`
/// runs once inside one transaction, which commits when it succeeds and
/// rolls back when it fails.
pub trait BoardRepository: Send + Sync {
    /// # Errors
    /// `work`'s refusal, unchanged, after rolling back; the store's own
    /// refusal (a missing board, contention).
    fn atomic(&self, create: bool, work: &mut BoardWork<'_>) -> Result<(), BoardError>;

    /// Runs `work`, which only reads, in one read transaction (#2338): a
    /// consistent view of the board that takes no write lock, so it
    /// neither waits for a writer's transaction nor holds one off (a
    /// writer's commit waits at most for the read transaction to finish).
    /// Every implementation refuses any write `work` attempts, and never
    /// creates a missing board. It adds no missing column
    /// (`ensure_columns` writes): `work` reads only the board's original
    /// schema, and a caller that may meet an older board falls back to
    /// [`Self::atomic`] on a store refusal. There is no default: a read
    /// that could write would be an `atomic` by another name.
    ///
    /// # Errors
    /// `work`'s refusal; the store's own (a missing board, contention, a
    /// write attempted).
    fn read(&self, work: &mut BoardWork<'_>) -> Result<(), BoardError>;
}

/// `uuid.uuid4().hex`: 32 lowercase hex digits, a fresh value per draw.
pub trait IdSource: Send + Sync {
    fn hex32(&self) -> String;
}

/// The board's `encode(value)`: `json.dumps` with sorted keys, compact
/// separators and `ensure_ascii`. The board bounds a JSON argument by the
/// text it would store before storing it.
pub trait BoardEncoding: Send + Sync {
    /// # Errors
    /// A value nested too deep to encode (Python raises `RecursionError`).
    fn encode(&self, value: &Value) -> Result<String, BoardError>;
}
