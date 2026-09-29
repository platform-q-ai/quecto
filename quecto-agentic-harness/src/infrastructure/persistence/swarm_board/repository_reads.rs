//! The read models' statements over the SQLite store (#2277, #1969), by
//! Python's SQL (`swarm.py`, `swarm_tasks.py`): the run as `dict(row)`,
//! the owners' statuses and their one grouped `max(time)` scan, an
//! event's time, the event page, the evidence rows, the task id page, the
//! task states `summary` counts and the owners of the held claims. Each
//! value is read as stored ([`cell_at`]); a JSON column is loaded as
//! Python's `json.loads` loads it, and one that is not JSON text is
//! refused as a store failure where Python raises.
use rusqlite::Row;
use rusqlite::types::Value as SqlValue;
use serde_json::Value;

use super::binding;
use super::repository::{ACTIVE_CLAIM, SqliteBoard, cell_at, failed, fetched, loose, text};
use super::repository_tasks::loaded;
use crate::application::swarm::dto::{CountedTask, DictRow, LatestActivity};
use crate::domain::swarm::BoardError;

/// The run's JSON columns `summary` loads.
const RUN_JSON_COLUMNS: [&str; 2] = ["constraints", "criteria"];

/// `dict(row)` of a fetched row, the columns named in `json` loaded.
fn dict(row: &Row<'_>, json: &[&str]) -> rusqlite::Result<DictRow> {
    fetched(row)?;
    let columns = (0..row.as_ref().column_count())
        .map(|index| {
            let name = row.as_ref().column_name(index)?.to_owned();
            let value = if json.contains(&name.as_str()) {
                loaded(row, index)?
            } else {
                cell_at(row, index)?
            };
            Ok((name, value))
        })
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(DictRow { columns })
}

/// `(?,?,…)` for `count` parameters.
fn marks(count: usize) -> String {
    debug_assert!(count > 0, "an IN list names at least one value");
    format!("({})", vec!["?"; count].join(","))
}

fn texts(values: &[&str]) -> Vec<SqlValue> {
    values
        .iter()
        .map(|value| SqlValue::Text((*value).to_owned()))
        .collect()
}

impl SqliteBoard<'_> {
    /// Every row `sql` selects with `parameters` bound, each read by
    /// `read`.
    fn rows_of<T>(
        &self,
        sql: &str,
        parameters: &[SqlValue],
        read: impl Fn(&Row<'_>) -> rusqlite::Result<T>,
    ) -> Result<Vec<T>, BoardError> {
        let mut statement =
            binding::bound_statement(self.connection, sql, parameters).map_err(failed)?;
        let mut rows = statement.raw_query();
        let mut found = Vec::new();
        while let Some(row) = rows.next().map_err(failed)? {
            found.push(read(row).map_err(failed)?);
        }
        Ok(found)
    }

    /// `SELECT * FROM run` as `dict(row)`, its contract loaded.
    pub(super) fn run_dict(&self) -> Result<Option<DictRow>, BoardError> {
        let rows = self.rows_of("SELECT * FROM run", &[], |row| {
            self.seen(row);
            dict(row, &RUN_JSON_COLUMNS)
        })?;
        Ok(rows.into_iter().next())
    }

    /// `SELECT id,status FROM members WHERE id IN (…)`.
    pub(super) fn statuses_of(
        &self,
        ids: &[&str],
    ) -> Result<Vec<(String, Option<String>)>, BoardError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let sql = format!(
            "SELECT id,status FROM members WHERE id IN {}",
            marks(ids.len())
        );
        let rows = self.rows_of(&sql, &texts(ids), |row| {
            fetched(row)?;
            Ok((text(row, "id")?, text(row, "status")?))
        })?;
        // `id IN (texts)` matches only text ids.
        Ok(rows
            .into_iter()
            .filter_map(|(id, status)| id.map(|id| (id, status)))
            .collect())
    }

    /// `SELECT actor, max(time) latest FROM events WHERE actor IN (…)
    /// GROUP BY actor`: the one grouped scan.
    pub(super) fn latest_of(&self, actors: &[&str]) -> Result<Vec<LatestActivity>, BoardError> {
        if actors.is_empty() {
            return Ok(Vec::new());
        }
        let sql = format!(
            "SELECT actor, max(time) latest FROM events WHERE actor IN {} GROUP BY actor",
            marks(actors.len())
        );
        let rows = self.rows_of(&sql, &texts(actors), |row| {
            fetched(row)?;
            Ok((text(row, "actor")?, cell_at(row, 1)?))
        })?;
        Ok(rows
            .into_iter()
            .filter_map(|(actor, latest)| actor.map(|actor| LatestActivity { actor, latest }))
            .collect())
    }

    /// `SELECT time FROM events WHERE id=?`.
    pub(super) fn time_of(&self, id: i64) -> Result<Option<Value>, BoardError> {
        let rows = self.rows_of(
            "SELECT time FROM events WHERE id=?",
            &[SqlValue::Integer(id)],
            |row| {
                fetched(row)?;
                cell_at(row, 0)
            },
        )?;
        Ok(rows.into_iter().next())
    }

    /// `SELECT * FROM events WHERE id>? ORDER BY id LIMIT ?`.
    pub(super) fn events_after(&self, after: u64, limit: i64) -> Result<Vec<DictRow>, BoardError> {
        self.rows_of(
            "SELECT * FROM events WHERE id>? ORDER BY id LIMIT ?",
            &[loose(1, &Value::from(after))?, SqlValue::Integer(limit)],
            |row| dict(row, &[]),
        )
    }

    /// `SELECT * FROM evidence`.
    pub(super) fn evidence_dicts(&self) -> Result<Vec<DictRow>, BoardError> {
        self.rows_of("SELECT * FROM evidence", &[], |row| dict(row, &[]))
    }

    /// `SELECT id FROM tasks ORDER BY id LIMIT ? OFFSET ?`.
    pub(super) fn ids_page(&self, offset: u64, limit: i64) -> Result<Vec<Value>, BoardError> {
        self.rows_of(
            "SELECT id FROM tasks ORDER BY id LIMIT ? OFFSET ?",
            &[SqlValue::Integer(limit), loose(2, &Value::from(offset))?],
            |row| {
                fetched(row)?;
                cell_at(row, 0)
            },
        )
    }

    /// `SELECT id,status,dependencies FROM tasks`.
    pub(super) fn states(&self) -> Result<Vec<CountedTask>, BoardError> {
        self.rows_of("SELECT id,status,dependencies FROM tasks", &[], |row| {
            fetched(row)?;
            Ok(CountedTask {
                id: cell_at(row, 0)?,
                status: cell_at(row, 1)?,
                dependencies: loaded(row, 2)?,
            })
        })
    }

    /// `SELECT id, owner, status FROM tasks WHERE owner IS NOT NULL AND
    /// status IN ('claimed','blocked','submitted')`: each owner.
    pub(super) fn owners_of_claims(&self) -> Result<Vec<Value>, BoardError> {
        self.rows_of(
            &format!(
                "SELECT id, owner, status FROM tasks WHERE owner IS NOT NULL AND status IN {ACTIVE_CLAIM}"
            ),
            &[],
            |row| {
                fetched(row)?;
                cell_at(row, 1)
            },
        )
    }
}
