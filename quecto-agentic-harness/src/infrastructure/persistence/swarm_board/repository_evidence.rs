//! `BoardEvidence` and the contract reads and writes of `BoardRuns` over
//! the SQLite store (#2273), by Python's SQL (`swarm_repository.py`,
//! `swarm.py`): what completion reads, the task evidence a revalidation
//! writes with plain `json.dumps`, the criterion evidence `evidence`
//! records and `amend` deletes, and the contract `amend` replaces.
//!
//! Rows are fetched as Python fetches them (every column decoded, text
//! that is not UTF-8 refused with Python's text). A contract column the
//! board never leaves unloadable (not JSON text) is refused as a store
//! failure where Python's `json.loads` raises, and a task's `acceptance`
//! and `dependencies` are loaded too, where `Transaction.task` loads only
//! its evidence (the `outside_edited_contract` divergence).
use rusqlite::types::Value as SqlValue;
use rusqlite::{OptionalExtension, Row, params};
use serde_json::Value;

use super::binding;
use super::py_json::{self, PyJson};
use super::repository::{SqliteBoard, cell_at, encoded, failed, fetched, loose};
use super::repository_tasks::{loaded, task_row};
use crate::application::swarm::dto::{
    AmendedContract, CompletionState, EvidenceEntry, NewEvidence, PriorEvidence, StoredContract,
};
use crate::application::swarm::ports::BoardEvidence;
use crate::domain::swarm::{BoardError, RefusalKind};

impl BoardEvidence for SqliteBoard<'_> {
    fn evidence_rows(&self) -> Result<Vec<crate::application::swarm::dto::DictRow>, BoardError> {
        self.evidence_dicts()
    }

    fn completion_state(&self) -> Result<CompletionState, BoardError> {
        let criteria = run_criteria(self)?
            .ok_or_else(|| BoardError::new(RefusalKind::RunMissing, "coordination run missing"))?;
        let evidence = self.rows("SELECT * FROM evidence", |row| {
            fetched(row)?;
            Ok(EvidenceEntry {
                criterion: cell(row, "criterion")?,
                artifact: cell(row, "artifact")?,
                revision: cell(row, "revision")?,
                kind: cell(row, "kind")?,
                accepted: cell(row, "accepted")?,
            })
        })?;
        // `SELECT id FROM tasks` reads the INTEGER PRIMARY KEY in order.
        let tasks = self.rows("SELECT * FROM tasks ORDER BY id", task_row)?;
        let has_reservations = self
            .connection
            .query_row("SELECT 1 FROM files", [], |_| Ok(()))
            .optional()
            .map_err(failed)?
            .is_some();
        Ok(CompletionState {
            criteria,
            evidence,
            tasks,
            has_reservations,
        })
    }

    fn replace_task_evidence(&self, id: &Value, evidence: &Value) -> Result<(), BoardError> {
        let evidence = PyJson::try_from(evidence)
            .map_err(|error| error.to_string())
            .and_then(|value| py_json::dumps(&value).map_err(|error| error.to_string()))
            .map_err(|error| BoardError::new(RefusalKind::Invalid, error))?;
        self.run_on_task(
            "UPDATE tasks SET evidence=? WHERE id=?",
            &[SqlValue::Text(evidence), loose(2, id)?],
        )
    }

    fn delete_all_evidence(&self) -> Result<(), BoardError> {
        // Any number of rows, none included.
        self.run("DELETE FROM evidence", &[]).map(|_| ())
    }

    fn prior_evidence(
        &self,
        criterion: &Value,
        actor: &str,
    ) -> Result<Option<PriorEvidence>, BoardError> {
        let parameters = [loose(1, criterion)?, SqlValue::Text(actor.to_owned())];
        let mut statement = binding::bound_statement(
            self.connection,
            "SELECT artifact,revision,kind,accepted FROM evidence WHERE criterion=? AND actor=?",
            &parameters,
        )
        .map_err(failed)?;
        let mut rows = statement.raw_query();
        rows.next()
            .and_then(|row| {
                row.map(|row| {
                    fetched(row)?;
                    Ok(PriorEvidence {
                        artifact: cell(row, "artifact")?,
                        revision: cell(row, "revision")?,
                        kind: cell(row, "kind")?,
                        accepted: cell(row, "accepted")?,
                    })
                })
                .transpose()
            })
            .map_err(failed)
    }

    fn record_evidence(&self, evidence: &NewEvidence) -> Result<(), BoardError> {
        let written = self.run(
            "INSERT OR REPLACE INTO evidence VALUES(?,?,?,?,?,?)",
            &[
                loose(1, &evidence.criterion)?,
                SqlValue::Text(evidence.artifact.clone()),
                SqlValue::Text(evidence.revision.clone()),
                SqlValue::Text(evidence.kind.clone()),
                SqlValue::Text(evidence.actor.clone()),
                SqlValue::Integer(i64::from(evidence.accepted)),
            ],
        )?;
        debug_assert_eq!(written, 1, "one VALUES row is written");
        Ok(())
    }
}

impl SqliteBoard<'_> {
    /// Every row `sql` selects, each read by `read`, in order.
    fn rows<T>(
        &self,
        sql: &str,
        read: impl FnMut(&Row<'_>) -> rusqlite::Result<T>,
    ) -> Result<Vec<T>, BoardError> {
        let mut statement = self.connection.prepare(sql).map_err(failed)?;
        let rows = statement.query_map([], read).map_err(failed)?;
        rows.collect::<rusqlite::Result<_>>().map_err(failed)
    }
}

/// The column `name` of `row` as the JSON value of what it stores.
fn cell(row: &Row<'_>, name: &str) -> rusqlite::Result<Value> {
    cell_at(row, row.as_ref().column_index(name)?)
}

/// `json.loads(run['criteria'])`, when the store holds a run.
pub(super) fn run_criteria(board: &SqliteBoard<'_>) -> Result<Option<Value>, BoardError> {
    board
        .connection
        .query_row("SELECT * FROM run", [], |row| {
            fetched(row)?;
            loaded(row, row.as_ref().column_index("criteria")?)
        })
        .optional()
        .map_err(failed)
}

/// The run's goal as stored, and its constraints and criteria loaded.
pub(super) fn run_contract(board: &SqliteBoard<'_>) -> Result<Option<StoredContract>, BoardError> {
    board
        .connection
        .query_row("SELECT * FROM run", [], |row| {
            fetched(row)?;
            Ok(StoredContract {
                goal: cell(row, "goal")?,
                constraints: loaded(row, row.as_ref().column_index("constraints")?)?,
                criteria: loaded(row, row.as_ref().column_index("criteria")?)?,
            })
        })
        .optional()
        .map_err(failed)
}

/// `UPDATE run SET goal=?,constraints=?,criteria=?`, on the run row the
/// operation gate has read in this transaction.
pub(super) fn amend_contract(
    board: &SqliteBoard<'_>,
    contract: &AmendedContract,
) -> Result<(), BoardError> {
    let changed = board
        .connection
        .execute(
            "UPDATE run SET goal=?,constraints=?,criteria=?",
            params![
                contract.goal,
                encoded(&contract.constraints)?,
                encoded(&contract.criteria)?,
            ],
        )
        .map_err(failed)?;
    debug_assert!(
        changed >= 1,
        "the amendment changes the run row the gate read"
    );
    Ok(())
}
