//! The coordination board's file reservations, and the checkout paths they
//! normalise.

use crate::application::swarm::dto::{FileRow, NewReservation};
use crate::domain::swarm::value_objects::board_error::BoardError;
use serde_json::Value;

/// The `files` reservations (#2272, #2275). A task id, a claim token or
/// an ownership token is the caller's value, bound as Python's `sqlite3`
/// binds it.
pub trait BoardFiles {
    /// `DELETE FROM files WHERE task=? AND claim=?`: the reservations made
    /// under that claim of that task, and no other, each bound as given.
    fn delete_claim_files(&self, task: &Value, claim: &Value) -> Result<(), BoardError>;
    /// `SELECT count(*) FROM files`.
    fn file_count(&self) -> Result<i64, BoardError>;
    /// Whether `path` is reserved (`SELECT 1 FROM files WHERE path=?`).
    fn file_reserved(&self, path: &str) -> Result<bool, BoardError>;
    /// `INSERT INTO files VALUES(?,?,?,?,?)`, one row per path in the
    /// order given.
    fn insert_files(&self, reservation: &NewReservation) -> Result<(), BoardError>;
    /// `DELETE FROM files WHERE task=? AND owner=? AND claim=? AND
    /// token=?`: one reservation set of one claim, when it is there; how
    /// many files it released (#2394 round-1 review L1), none included.
    fn delete_reservation(
        &self,
        task: &Value,
        owner: &str,
        claim: &Value,
        token: &Value,
    ) -> Result<usize, BoardError>;
    /// `SELECT count(*) FROM files WHERE task=?`.
    fn task_file_count(&self, task: &Value) -> Result<i64, BoardError>;
    /// `DELETE FROM files WHERE task=?`: every reservation of the task,
    /// whichever claim made it.
    fn delete_task_files(&self, task: &Value) -> Result<(), BoardError>;
    /// `SELECT * FROM files ORDER BY path LIMIT ? OFFSET ?`, each row as
    /// `dict(row)`. An offset beyond SQLite's integers is refused as the
    /// store refuses an integer it cannot bind.
    fn file_page(&self, offset: u64, limit: i64) -> Result<Vec<FileRow>, BoardError>;
    /// `SELECT count(*) FROM files WHERE owner=?` (#2277), the owner bound
    /// as given.
    fn owner_file_count(&self, owner: &Value) -> Result<i64, BoardError>;
    /// `DELETE FROM files WHERE owner=?` (#2277): every reservation of a
    /// member whose orderly exit is confirmed.
    fn delete_owner_files(&self, owner: &Value) -> Result<(), BoardError>;
}

/// The shared checkout a board's file reservations name (#2275).
/// Normalising a path reads the filesystem (it follows symlinks), so it is
/// an effect behind this port; the adapter is bound to one checkout.
pub trait CheckoutPaths: Send + Sync {
    /// `str((root / path).resolve().relative_to(root))` with `root` the
    /// checkout resolved: Python's non-strict `Path.resolve()`, which
    /// follows the symlinks that exist and keeps a missing remainder as
    /// written (`..` taken lexically), then the path relative to the root
    /// (`.` for the root itself).
    ///
    /// # Errors
    /// `file must resolve inside the shared checkout` for a path that
    /// resolves outside it, or cannot be resolved.
    fn normalize(&self, path: &str) -> Result<String, BoardError>;
}
