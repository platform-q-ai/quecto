//! The loss and death statements over the SQLite store (#2277, #1961), by
//! Python's SQL (`swarm.py`): the launcher read and the loss scan of one
//! member, the loss observations, a dead member's work blocked, its
//! reservations counted and deleted by owner, and the failed hold of an
//! outcome-less pause. The member is the caller's value, bound as
//! Python's `sqlite3` binds it ([`loose`]) and numbered by its position in
//! Python's statement.
//!
//! An observation's detail is loaded as Python's `json.loads` loads it (a
//! detail that is not JSON text is refused as a store failure where
//! Python raises; one that is JSON but not an object names no member,
//! where Python's `.get` raises), and its time is read as stored
//! ([`cell_at`]): a BLOB,
//! which only an edit writes, is refused as a store failure where Python
//! passes over an observation of another member (the
//! `outside_edited_loss_records` divergence).
use rusqlite::types::Value as SqlValue;
use serde_json::Value;

use super::binding;
use super::repository::{ACTIVE_CLAIM, SqliteBoard, cell_at, failed, fetched, loose, text};
use super::repository_control::lost_among;
use super::repository_tasks::loaded;
use crate::application::swarm::dto::ScopeObservation;
use crate::domain::swarm::BoardError;

impl SqliteBoard<'_> {
    /// `SELECT launcher FROM members WHERE id=?`.
    pub(super) fn launcher_of(&self, id: &Value) -> Result<Option<Option<String>>, BoardError> {
        let parameters = [loose(1, id)?];
        let mut statement = binding::bound_statement(
            self.connection,
            "SELECT launcher FROM members WHERE id=?",
            &parameters,
        )
        .map_err(failed)?;
        let mut rows = statement.raw_query();
        rows.next()
            .and_then(|row| {
                row.map(|row| {
                    fetched(row)?;
                    text(row, "launcher")
                })
                .transpose()
            })
            .map_err(failed)
    }

    /// `lost_after_activation(connection, member)`.
    pub(super) fn lost_since_activation(&self, member: &Value) -> Result<bool, BoardError> {
        let lost = lost_among(self.connection, std::slice::from_ref(member))?;
        debug_assert_eq!(lost.len(), 1, "one member asked about");
        Ok(lost.first().copied().unwrap_or(false))
    }

    /// `SELECT actor, time, detail FROM events WHERE
    /// action='scope_observed' ORDER BY id`.
    pub(super) fn observations(&self) -> Result<Vec<ScopeObservation>, BoardError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT actor, time, detail FROM events WHERE action='scope_observed' ORDER BY id",
            )
            .map_err(failed)?;
        let rows = statement
            .query_map([], |row| {
                fetched(row)?;
                let detail = loaded(row, 2)?;
                Ok(ScopeObservation {
                    actor: text(row, "actor")?,
                    time: cell_at(row, 1)?,
                    member: detail.get("member").cloned().unwrap_or(Value::Null),
                })
            })
            .map_err(failed)?;
        rows.collect::<rusqlite::Result<_>>().map_err(failed)
    }

    /// `UPDATE tasks SET status='blocked',blocker=? WHERE owner=? AND
    /// status IN ('claimed','blocked','submitted')`: any number of tasks,
    /// none included.
    pub(super) fn block_owned(&self, owner: &Value, blocker: &str) -> Result<(), BoardError> {
        self.run(
            &format!(
                "UPDATE tasks SET status='blocked',blocker=? WHERE owner=? AND status IN {ACTIVE_CLAIM}"
            ),
            &[SqlValue::Text(blocker.to_owned()), loose(2, owner)?],
        )
        .map(|_| ())
    }

    /// `SELECT count(*) FROM files WHERE owner=?`.
    pub(super) fn owned_file_count(&self, owner: &Value) -> Result<i64, BoardError> {
        self.bound_count(
            "SELECT count(*) FROM files WHERE owner=?",
            &[loose(1, owner)?],
        )
    }

    /// `DELETE FROM files WHERE owner=?`: any number of reservations.
    pub(super) fn delete_owned_files(&self, owner: &Value) -> Result<(), BoardError> {
        self.run("DELETE FROM files WHERE owner=?", &[loose(1, owner)?])
            .map(|_| ())
    }

    /// `UPDATE run SET outcome='failed', outcome_reason=?` on the run row
    /// the operation gate has read in this transaction.
    pub(super) fn failed_hold(&self, reason: &str) -> Result<(), BoardError> {
        let changed = self.run(
            "UPDATE run SET outcome='failed', outcome_reason=?",
            &[SqlValue::Text(reason.to_owned())],
        )?;
        debug_assert!(changed >= 1, "the hold changes the run row the gate read");
        Ok(())
    }
}
