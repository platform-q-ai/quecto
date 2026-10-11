//! `BoardProjectionReads` over the SQLite store (#2478): the supervising
//! session's projection, by plain SELECTs of allowlisted columns only.
//! No `SELECT *`: a token, a member's pid, socket or launcher, or a column
//! a later schema adds is never read. Nothing is created or altered, so it
//! runs in a read transaction: a board an older writer laid out reads its
//! missing columns as NULL, and one with no usage ledger as zero totals.
//! A TEXT cell that is not UTF-8 is refused naming its table and column
//! (`tasks title`), never
//! echoing the bytes.
use std::collections::BTreeSet;

use rusqlite::{Row, params};
use serde_json::Value;

use super::repository::{SqliteBoard, failed};
use super::repository_tasks::loaded;
use crate::application::swarm::dto::{
    ProjectionLimits, ProjectionRows, StoredEvidenceColumns, StoredItemColumns,
    StoredMemberColumns, StoredRunColumns,
};
use crate::application::swarm::ports::{BoardEvents, BoardProjectionReads};
use crate::domain::swarm::{BoardError, RefusalKind};

impl BoardProjectionReads for SqliteBoard<'_> {
    fn projection_revision(&self) -> Result<i64, BoardError> {
        let events = self.count("SELECT coalesce(max(id),0) FROM events")?;
        let usage = match self.table("request_usage")? {
            true => self.count("SELECT coalesce(max(rowid),0) FROM request_usage")?,
            false => 0,
        };
        events
            .checked_add(usage)
            .ok_or_else(|| BoardError::new(RefusalKind::Store, "board revision out of range"))
    }

    fn projection_rows(&self, limits: ProjectionLimits) -> Result<ProjectionRows, BoardError> {
        let (budget, observed_tokens, unknown_usage_requests) = self.usage()?;
        Ok(ProjectionRows {
            revision: self.projection_revision()?,
            generation: self.control_generation()?,
            run: self.projected_run()?,
            items: self.projected(
                "SELECT id, title, acceptance, dependencies, status, owner, blocker, evidence FROM tasks ORDER BY id LIMIT ?",
                limits.items,
                item,
            )?,
            item_count: self.count("SELECT count(*) FROM tasks")?,
            files: self.projected("SELECT path, task FROM files ORDER BY path LIMIT ?", limits.files, |row| {
                Ok((cell(row, 0, "files path")?, cell(row, 1, "files task")?))
            })?,
            file_count: self.count("SELECT count(*) FROM files")?,
            members: self.projected(
                "SELECT m.id, m.status, (SELECT max(e.time) FROM events e WHERE e.actor=m.id) FROM members m ORDER BY m.id LIMIT ?",
                limits.members,
                |row| {
                    Ok(StoredMemberColumns {
                        id: cell(row, 0, "members id")?,
                        status: cell(row, 1, "members status")?,
                        last_activity: cell(row, 2, "events time")?,
                    })
                },
            )?,
            member_count: self.count("SELECT count(*) FROM members")?,
            evidence: self.projected(
                "SELECT criterion, artifact, revision, kind, actor, accepted FROM evidence ORDER BY criterion, actor LIMIT ?",
                limits.evidence,
                evidence,
            )?,
            evidence_count: self.count("SELECT count(*) FROM evidence")?,
            budget,
            observed_tokens,
            unknown_usage_requests,
        })
    }
}

impl SqliteBoard<'_> {
    fn table(&self, name: &str) -> Result<bool, BoardError> {
        self.connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?",
                [name],
                |row| row.get::<_, i64>(0),
            )
            .map(|count| count > 0)
            .map_err(failed)
    }

    fn projected<T>(
        &self,
        sql: &str,
        limit: i64,
        read: impl Fn(&Row<'_>) -> rusqlite::Result<T>,
    ) -> Result<Vec<T>, BoardError> {
        let mut statement = self.connection.prepare(sql).map_err(failed)?;
        let rows = statement
            .query_map(params![limit], |row| read(row))
            .map_err(failed)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(failed)
    }

    /// The run's allowlisted columns, the two an older writer's board
    /// lacks read as NULL.
    fn projected_run(&self) -> Result<Option<StoredRunColumns>, BoardError> {
        let mut statement = self
            .connection
            .prepare("SELECT name FROM pragma_table_info('run')")
            .map_err(failed)?;
        let columns: BTreeSet<String> = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(failed)?
            .collect::<rusqlite::Result<_>>()
            .map_err(failed)?;
        let optional = |name: &str| match columns.contains(name) {
            true => name.to_owned(),
            false => "NULL".to_owned(),
        };
        let sql = format!(
            "SELECT id, goal, criteria, coordinator, integrator, member_limit, deadline, status, {}, {} FROM run",
            optional("outcome"),
            optional("outcome_reason")
        );
        let mut statement = self.connection.prepare(&sql).map_err(failed)?;
        let mut rows = statement
            .query_map([], |row| {
                Ok(StoredRunColumns {
                    id: cell(row, 0, "run id")?,
                    goal: cell(row, 1, "run goal")?,
                    criteria: json(row, 2)?,
                    coordinator: cell(row, 3, "run coordinator")?,
                    integrator: cell(row, 4, "run integrator")?,
                    member_limit: cell(row, 5, "run member_limit")?,
                    deadline: cell(row, 6, "run deadline")?,
                    status: cell(row, 7, "run status")?,
                    outcome: cell(row, 8, "run outcome")?,
                    outcome_reason: cell(row, 9, "run outcome_reason")?,
                })
            })
            .map_err(failed)?;
        rows.next().transpose().map_err(failed)
    }

    /// The budget's payload and the ledger's two totals: none and zero
    /// when the tables were never made.
    fn usage(&self) -> Result<(Option<Value>, Value, Value), BoardError> {
        let budget = match self.table("usage_budget")? {
            true => self
                .projected(
                    "SELECT payload FROM usage_budget WHERE id=1 LIMIT ?",
                    1,
                    |row| json(row, 0),
                )?
                .pop(),
            false => None,
        };
        let totals = match self.table("request_usage")? {
            true => self.connection.query_row(
                "SELECT coalesce(sum(tokens),0), coalesce(sum(unknown),0) FROM request_usage",
                [],
                |row| {
                    Ok((
                        cell(row, 0, "request_usage tokens")?,
                        cell(row, 1, "request_usage unknown")?,
                    ))
                },
            ),
            false => Ok((Value::from(0), Value::from(0))),
        }
        .map_err(failed)?;
        Ok((budget, totals.0, totals.1))
    }
}

fn item(row: &Row<'_>) -> rusqlite::Result<StoredItemColumns> {
    Ok(StoredItemColumns {
        id: cell(row, 0, "tasks id")?,
        title: cell(row, 1, "tasks title")?,
        criteria: json(row, 2)?,
        depends_on: json(row, 3)?,
        status: cell(row, 4, "tasks status")?,
        owner: cell(row, 5, "tasks owner")?,
        blocker: cell(row, 6, "tasks blocker")?,
        evidence: json(row, 7)?,
    })
}

fn evidence(row: &Row<'_>) -> rusqlite::Result<StoredEvidenceColumns> {
    Ok(StoredEvidenceColumns {
        criterion: cell(row, 0, "evidence criterion")?,
        artifact: cell(row, 1, "evidence artifact")?,
        revision: cell(row, 2, "evidence revision")?,
        kind: cell(row, 3, "evidence kind")?,
        actor: cell(row, 4, "evidence actor")?,
        accepted: cell(row, 5, "evidence accepted")?,
    })
}

/// A JSON column the board writes as text; NULL as null.
fn json(row: &Row<'_>, index: usize) -> rusqlite::Result<Value> {
    match row.get_ref(index)? {
        rusqlite::types::ValueRef::Null => Ok(Value::Null),
        _ => loaded(row, index),
    }
}

/// A cell as the JSON value of what it stores. A BLOB, a REAL JSON cannot
/// hold, or TEXT that is not UTF-8 is refused naming the column only.
fn cell(row: &Row<'_>, index: usize, column: &str) -> rusqlite::Result<Value> {
    use rusqlite::types::ValueRef;
    let refused = |what: &str| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            format!("{column} holds {what}").into(),
        )
    };
    match row.get_ref(index)? {
        ValueRef::Null => Ok(Value::Null),
        ValueRef::Integer(integer) => Ok(Value::from(integer)),
        ValueRef::Real(real) => serde_json::Number::from_f64(real)
            .map(Value::Number)
            .ok_or_else(|| refused("a number JSON cannot hold")),
        ValueRef::Text(bytes) => std::str::from_utf8(bytes)
            .map(|text| Value::String(text.to_owned()))
            .map_err(|_| refused("text that is not UTF-8")),
        ValueRef::Blob(_) => Err(refused("bytes")),
    }
}
