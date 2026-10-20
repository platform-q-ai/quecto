//! The coordination board's read-only projection.

use crate::application::swarm::dto::{ProjectionLimits, ProjectionRows, StoredRevision};
use crate::domain::swarm::value_objects::board_error::BoardError;

/// The supervising session's projection reads (#2478): allowlisted columns
/// and bounded rows by plain SELECTs, in a read transaction, creating and
/// altering nothing; an older writer's board reads as it is.
pub trait BoardProjectionReads {
    /// The board's revision (the latest event's id plus the usage ledger's
    /// latest row id), which every write the projection shows moves, and
    /// the id of the run it counts.
    ///
    /// # Errors
    /// The store's refusal; a run id longer than the board writes.
    fn projection_revision(&self) -> Result<StoredRevision, BoardError>;

    /// The allowlisted columns, at most `limits` rows of each kind, copied
    /// only while their stored text (measured first) fits `limits.bytes`.
    ///
    /// # Errors
    /// The store's refusal; a value no column type can hold; a run or an
    /// item's state larger than the board writes.
    fn projection_rows(&self, limits: ProjectionLimits) -> Result<ProjectionRows, BoardError>;
}
