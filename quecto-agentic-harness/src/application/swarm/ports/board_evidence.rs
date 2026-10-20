//! The coordination board's criterion and task evidence (#2273).

use crate::application::swarm::dto::{CompletionState, DictRow, NewEvidence, PriorEvidence};
use crate::domain::swarm::value_objects::board_error::BoardError;
use serde_json::Value;

/// The criterion evidence and the task evidence completion reads, and
/// the criterion evidence `evidence` records (#2273).
pub trait BoardEvidence {
    /// `Transaction.completion_state()`: the run's criteria, every
    /// evidence row, every task as `Transaction.task` reads it, and
    /// whether a file reservation is left.
    fn completion_state(&self) -> Result<CompletionState, BoardError>;
    /// `UPDATE tasks SET evidence=? WHERE id=?` with the evidence written
    /// by plain `json.dumps` (not the board's `encode()`), the id bound as
    /// Python's `sqlite3` binds it.
    fn replace_task_evidence(&self, id: &Value, evidence: &Value) -> Result<(), BoardError>;
    /// `DELETE FROM evidence`: an amended contract keeps no evidence.
    fn delete_all_evidence(&self) -> Result<(), BoardError>;
    /// The row `actor` recorded for `criterion`, bound as given.
    fn prior_evidence(
        &self,
        criterion: &Value,
        actor: &str,
    ) -> Result<Option<PriorEvidence>, BoardError>;
    /// `INSERT OR REPLACE INTO evidence VALUES(?,?,?,?,?,?)`.
    fn record_evidence(&self, evidence: &NewEvidence) -> Result<(), BoardError>;
    /// `SELECT * FROM evidence` (#2277), each row as `dict(row)`.
    fn evidence_rows(&self) -> Result<Vec<DictRow>, BoardError>;
}
