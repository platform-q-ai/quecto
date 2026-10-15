//! `BoardProjectionReads` over the SQLite store (#2478): the supervising
//! session's projection, by plain SELECTs of allowlisted columns only.
//! No `SELECT *`: a token, a member's pid, socket or launcher, or a column
//! a later schema adds is never read. Nothing is created or altered, so it
//! runs in a read transaction: a board an older writer laid out reads its
//! missing columns as NULL, and one with no usage ledger as zero totals.
//! A TEXT cell that is not UTF-8, or a JSON cell the board could not have
//! written, is refused naming its table and column (`tasks title`), never
//! echoing its content.
//!
//! Bounded (#2487 round 2): each kind's rows are measured first
//! (`octet_length`, which reads no content) and copied only while their
//! stored text stays within the limits' bytes, so an edited board with
//! huge cells is never loaded whole. Every item's state is read, under a
//! guard on its cells' sizes, and the run is refused when its text alone
//! passes the bytes.
use std::collections::BTreeSet;

use rusqlite::{Row, params};
use serde_json::Value;

use super::repository::{SqliteBoard, failed};
use super::repository_tasks::loaded;
use crate::application::swarm::dto::{
    ProjectionLimits, ProjectionRows, StoredEvidenceColumns, StoredItemColumns,
    StoredMemberColumns, StoredRevision, StoredRunColumns, StoredStateColumns,
};
use crate::application::swarm::ports::{BoardEvents, BoardProjectionReads};
use crate::domain::swarm::validation::MEMBER_ID_MAX_BYTES;
use crate::domain::swarm::{BoardError, DEPENDENCIES_MAX, RefusalKind};

/// The most bytes an item's stored status holds (the longest the board
/// writes is `submitted`).
const STATUS_BYTES: i64 = 16;
/// The most bytes an item's stored dependencies hold: at most
/// [`DEPENDENCIES_MAX`] ids, each at most 20 digits and a separator.
const DEPENDENCIES_BYTES: i64 = DEPENDENCIES_MAX as i64 * 24 + 2;
/// The most bytes a usage budget's stored settings hold.
const BUDGET_BYTES: i64 = 4096;
/// The most bytes a run id or a member id holds.
const ID_BYTES: i64 = MEMBER_ID_MAX_BYTES as i64;
/// The most bytes a scalar cell (a number, a flag) holds as text: far more
/// than any number the board writes (#2487 round 3).
const SCALAR_BYTES: i64 = 64;
/// A member's latest board event's time: its last activity.
const LATEST_EVENT_TIME: &str =
    "(SELECT e.time FROM events e WHERE e.actor=m.id ORDER BY e.id DESC LIMIT 1)";

/// Two result columns for the scalar `expr`: whether it is within
/// [`SCALAR_BYTES`], and its value only when it is, so a longer one is
/// never copied out ([`bounded`] refuses it).
fn guarded(expr: &str) -> String {
    format!(
        "coalesce(octet_length({expr}),0) <= {SCALAR_BYTES}, CASE WHEN coalesce(octet_length({expr}),0) <= {SCALAR_BYTES} THEN {expr} END"
    )
}

/// The [`guarded`] cell at `index` (its flag) and `index + 1` (its value):
/// refused naming `column` when it was longer than the board writes.
fn bounded(row: &Row<'_>, index: usize, column: &str) -> rusqlite::Result<Value> {
    match row.get::<_, bool>(index)? {
        true => cell(row, index + 1, column),
        false => Err(rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            format!("{column} holds more than this board writes").into(),
        )),
    }
}

/// The stored text of one item row, as the items read copies it.
const ITEM_BYTES: &str = "coalesce(octet_length(title),0) + coalesce(octet_length(acceptance),0) + coalesce(octet_length(dependencies),0) + coalesce(octet_length(status),0) + coalesce(octet_length(owner),0) + coalesce(octet_length(blocker),0) + coalesce(octet_length(evidence),0)";
/// A member row's stored text, its latest event's time included.
const MEMBER_BYTES: &str = "coalesce(octet_length(m.id),0) + coalesce(octet_length(m.status),0) + coalesce(octet_length((SELECT e.time FROM events e WHERE e.actor=m.id ORDER BY e.id DESC LIMIT 1)),0)";
const EVIDENCE_BYTES: &str = "coalesce(octet_length(criterion),0) + coalesce(octet_length(artifact),0) + coalesce(octet_length(revision),0) + coalesce(octet_length(kind),0) + coalesce(octet_length(actor),0) + coalesce(octet_length(accepted),0)";

impl BoardProjectionReads for SqliteBoard<'_> {
    fn projection_revision(&self) -> Result<StoredRevision, BoardError> {
        let events = self.count("SELECT coalesce(max(id),0) FROM events")?;
        let usage = match self.table("request_usage")? {
            true => self.count("SELECT coalesce(max(rowid),0) FROM request_usage")?,
            false => 0,
        };
        let revision = events
            .checked_add(usage)
            .ok_or_else(|| BoardError::new(RefusalKind::Store, "board revision out of range"))?;
        let (run, deadline) = self.run_identity()?;
        Ok(StoredRevision {
            run,
            deadline,
            revision,
        })
    }

    fn projection_rows(&self, limits: ProjectionLimits) -> Result<ProjectionRows, BoardError> {
        let (budget, observed_tokens, unknown_usage_requests) = self.usage()?;
        let mut bytes = limits.bytes;
        let run = self.projected_run(&mut bytes)?;
        // Items first: their reservations never starve them (round 3 L4).
        let items = self.measured(
            &format!("SELECT {ITEM_BYTES} FROM tasks ORDER BY id LIMIT ?"),
            "SELECT id, title, acceptance, dependencies, status, owner, blocker, evidence FROM tasks ORDER BY id LIMIT ?",
            limits.items,
            &mut bytes,
            item,
        )?;
        let files = self.measured(
            "SELECT coalesce(octet_length(path),0) + coalesce(octet_length(task),0) FROM files ORDER BY path LIMIT ?",
            &format!(
                "SELECT path, {} FROM files ORDER BY path LIMIT ?",
                guarded("task")
            ),
            limits.files,
            &mut bytes,
            |row| Ok((cell(row, 0, "files path")?, bounded(row, 1, "files task")?)),
        )?;
        let members = self.measured(
            &format!("SELECT {MEMBER_BYTES} FROM members m ORDER BY m.id LIMIT ?"),
            // The latest event's time is looked up once per member: the
            // listed rows are materialised, then guarded.
            &format!(
                "WITH listed AS MATERIALIZED (SELECT m.id AS id, m.status AS status, {LATEST_EVENT_TIME} AS latest FROM members m ORDER BY m.id LIMIT ?1) SELECT id, status, {} FROM listed ORDER BY id",
                guarded("latest")
            ),
            limits.members,
            &mut bytes,
            |row| {
                Ok(StoredMemberColumns {
                    id: cell(row, 0, "members id")?,
                    status: cell(row, 1, "members status")?,
                    last_activity: bounded(row, 2, "events time")?,
                })
            },
        )?;
        let evidence = self.measured(
            &format!("SELECT {EVIDENCE_BYTES} FROM evidence ORDER BY criterion, actor LIMIT ?"),
            &format!(
                "SELECT criterion, artifact, revision, kind, actor, {} FROM evidence ORDER BY criterion, actor LIMIT ?",
                guarded("accepted")
            ),
            limits.evidence,
            &mut bytes,
            evidence,
        )?;
        debug_assert!(bytes >= 0, "no row is copied past the bytes");
        let item_count = self.count("SELECT count(*) FROM tasks")?;
        Ok(ProjectionRows {
            revision: self.projection_revision()?.revision,
            generation: self.control_generation()?,
            run,
            items,
            item_count,
            states: self.projected_states(item_count, limits.items)?,
            files,
            file_count: self.count("SELECT count(*) FROM files")?,
            members,
            member_count: self.count("SELECT count(*) FROM members")?,
            evidence,
            evidence_count: self.count("SELECT count(*) FROM evidence")?,
            budget,
            observed_tokens,
            unknown_usage_requests,
        })
    }
}

/// The refusal of a run the board could not have written.
fn edited_run() -> BoardError {
    BoardError::new(
        RefusalKind::Store,
        "the board's run is not as the board writes it",
    )
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

    /// The rows `rows` reads, at most `limit`, while their stored text
    /// (`sizes`, in the same order, reading no content) stays within
    /// `bytes`, which they are taken from: rows from the first that would
    /// pass it are never copied.
    fn measured<T>(
        &self,
        sizes: &str,
        rows: &str,
        limit: i64,
        bytes: &mut i64,
        read: impl Fn(&Row<'_>) -> rusqlite::Result<T>,
    ) -> Result<Vec<T>, BoardError> {
        let mut fitting = 0;
        for size in self.projected(sizes, limit, |row| row.get::<_, i64>(0))? {
            match bytes.checked_sub(size) {
                Some(left) if left >= 0 && size >= 0 => {
                    *bytes = left;
                    fitting += 1;
                }
                Some(_) | None => break,
            }
        }
        let read = self.projected(rows, fitting, read)?;
        debug_assert!(read.len() as i64 <= fitting, "no row past the measure");
        Ok(read)
    }

    /// Every item's id, status, dependencies and owner, at most `limit`:
    /// refused, unread, when one of those cells holds more than the board
    /// writes.
    fn projected_states(
        &self,
        item_count: i64,
        limit: i64,
    ) -> Result<Vec<StoredStateColumns>, BoardError> {
        let bounded = self
            .connection
            .query_row(
                "SELECT count(*) FROM tasks WHERE coalesce(octet_length(status),0) <= ?1 AND coalesce(octet_length(dependencies),0) <= ?2 AND coalesce(octet_length(owner),0) <= ?3",
                params![STATUS_BYTES, DEPENDENCIES_BYTES, ID_BYTES],
                |row| row.get::<_, i64>(0),
            )
            .map_err(failed)?;
        if bounded != item_count {
            return Err(BoardError::new(
                RefusalKind::Store,
                "tasks status, dependencies or owner holds more than this board writes",
            ));
        }
        self.projected(
            "SELECT id, status, dependencies, owner FROM tasks ORDER BY id LIMIT ?",
            limit,
            |row| {
                Ok(StoredStateColumns {
                    id: cell(row, 0, "tasks id")?,
                    status: cell(row, 1, "tasks status")?,
                    depends_on: json(row, 2, "tasks dependencies")?,
                    owner: cell(row, 3, "tasks owner")?,
                })
            },
        )
    }

    /// The run row's id and deadline, each read only when it is no longer
    /// than the board writes; both null before a run row exists.
    fn run_identity(&self) -> Result<(Value, Value), BoardError> {
        let mut statement = self
            .connection
            .prepare(&format!(
                "SELECT coalesce(octet_length(id),0) <= ?1, CASE WHEN coalesce(octet_length(id),0) <= ?1 THEN id END, {} FROM run LIMIT 1",
                guarded("deadline")
            ))
            .map_err(failed)?;
        let mut rows = statement
            .query_map(params![ID_BYTES], |row| {
                Ok((
                    row.get::<_, bool>(0)?,
                    cell(row, 1, "run id")?,
                    bounded(row, 2, "run deadline")?,
                ))
            })
            .map_err(failed)?;
        match rows.next().transpose().map_err(failed)? {
            None => Ok((Value::Null, Value::Null)),
            Some((true, id, deadline)) => Ok((id, deadline)),
            Some((false, _, _)) => Err(edited_run()),
        }
    }

    /// The run's allowlisted columns, the two an older writer's board
    /// lacks read as NULL: refused, unread, when its text passes `bytes`,
    /// which it is taken from.
    fn projected_run(&self, bytes: &mut i64) -> Result<Option<StoredRunColumns>, BoardError> {
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
        let (outcome, outcome_reason) = (optional("outcome"), optional("outcome_reason"));
        let sizes = format!(
            "SELECT coalesce(octet_length(id),0) + coalesce(octet_length(goal),0) + coalesce(octet_length(criteria),0) + coalesce(octet_length(coordinator),0) + coalesce(octet_length(integrator),0) + coalesce(octet_length(status),0) + coalesce(octet_length(member_limit),0) + coalesce(octet_length(deadline),0) + coalesce(octet_length({outcome}),0) + coalesce(octet_length({outcome_reason}),0) FROM run LIMIT ?"
        );
        let Some(size) = self.projected(&sizes, 1, |row| row.get::<_, i64>(0))?.pop() else {
            return Ok(None);
        };
        *bytes = match bytes.checked_sub(size) {
            Some(left) if left >= 0 && size >= 0 => left,
            Some(_) | None => return Err(edited_run()),
        };
        let sql = format!(
            "SELECT id, goal, criteria, coordinator, integrator, {}, {}, status, {outcome}, {outcome_reason} FROM run",
            guarded("member_limit"),
            guarded("deadline")
        );
        let mut statement = self.connection.prepare(&sql).map_err(failed)?;
        let mut rows = statement
            .query_map([], |row| {
                Ok(StoredRunColumns {
                    id: cell(row, 0, "run id")?,
                    goal: cell(row, 1, "run goal")?,
                    criteria: json(row, 2, "run criteria")?,
                    coordinator: cell(row, 3, "run coordinator")?,
                    integrator: cell(row, 4, "run integrator")?,
                    member_limit: bounded(row, 5, "run member_limit")?,
                    deadline: bounded(row, 7, "run deadline")?,
                    status: cell(row, 9, "run status")?,
                    outcome: cell(row, 10, "run outcome")?,
                    outcome_reason: cell(row, 11, "run outcome_reason")?,
                })
            })
            .map_err(failed)?;
        rows.next().transpose().map_err(failed)
    }

    /// The budget's payload and the ledger's two totals: none and zero
    /// when the tables were never made. A payload longer than the board
    /// writes (three settings) is left unread, as none.
    fn usage(&self) -> Result<(Option<Value>, Value, Value), BoardError> {
        // No budget is applied (owner decision, 2026-10-14): a payload no
        // board writes (past the bound, NULL or not JSON) reads as none,
        // never refusing the projection (review round 1, #2569).
        let budget = match self.table("usage_budget")? {
            true => self
                .projected(
                    "SELECT coalesce(octet_length(payload),0) <= ?1, CASE WHEN coalesce(octet_length(payload),0) <= ?1 THEN payload END FROM usage_budget WHERE id=1",
                    BUDGET_BYTES,
                    |row| Ok((row.get::<_, bool>(0)?, json(row, 1, "usage_budget payload").ok())),
                )?
                .pop()
                .and_then(|(bounded, payload)| match bounded {
                    true => payload,
                    false => None,
                }),
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
        criteria: json(row, 2, "tasks acceptance")?,
        depends_on: json(row, 3, "tasks dependencies")?,
        status: cell(row, 4, "tasks status")?,
        owner: cell(row, 5, "tasks owner")?,
        blocker: cell(row, 6, "tasks blocker")?,
        evidence: json(row, 7, "tasks evidence")?,
    })
}

fn evidence(row: &Row<'_>) -> rusqlite::Result<StoredEvidenceColumns> {
    Ok(StoredEvidenceColumns {
        criterion: cell(row, 0, "evidence criterion")?,
        artifact: cell(row, 1, "evidence artifact")?,
        revision: cell(row, 2, "evidence revision")?,
        kind: cell(row, 3, "evidence kind")?,
        actor: cell(row, 4, "evidence actor")?,
        accepted: bounded(row, 5, "evidence accepted")?,
    })
}

/// A JSON column the board writes as text; NULL as null. One the board
/// could not have written is refused naming its column only: the decoder's
/// own error may quote what it read (#2487 round 2, L3).
fn json(row: &Row<'_>, index: usize, column: &str) -> rusqlite::Result<Value> {
    match row.get_ref(index)? {
        rusqlite::types::ValueRef::Null => Ok(Value::Null),
        _ => loaded(row, index).map_err(|_| {
            rusqlite::Error::FromSqlConversionFailure(
                index,
                rusqlite::types::Type::Text,
                format!("{column} is not JSON this board writes").into(),
            )
        }),
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
