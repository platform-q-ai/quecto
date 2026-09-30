//! `BoardUsage` over the SQLite store (#2273, #2274): the usage report,
//! the budget and the request ledger by Python's SQL
//! (`swarm_repository.Transaction`), copied verbatim. The budget payload
//! is written by plain `json.dumps` and a request record as the text the
//! use case encoded with the board's `encode()` (and bounded), as Python
//! writes them. The two tables are created lazily,
//! inside the transaction and in Python's order, so a board either
//! implementation read lists them at the same `sqlite_master` positions;
//! the aggregates' column aliases are the report's JSON keys, and
//! `count(cache_read_tokens)` counts the measured requests alone, so the
//! SQL computes them, never Rust.
//!
//! The exception is what every recorded request and control receipt
//! reads (#2340): the ledger's row count and the budget's two totals come
//! from the sums the process keeps between transactions
//! ([`usage_sums`](super::usage_sums)), extended by the rows since, so
//! they cost the same however long the run; the SQL sums them only when a
//! row holds a value only an edit writes.
use rusqlite::{OptionalExtension, Row, params};
use serde_json::{Value, json};

use super::py_json::{self, PyJson};
use super::repository::{SqliteBoard, cell_at, failed, fetched};
use super::repository_tasks::loaded;
use super::usage_sums::{self, UsageSums};
use crate::application::swarm::dto::{
    NewRequestUsage, RecentRequest, StoredRequestUsage, UsageReport, UsageRow, UsageStanding,
};
use crate::application::swarm::ports::BoardUsage;
use crate::domain::swarm::{BoardError, RefusalKind};

/// `Transaction._usage_schema`, statement by statement.
const USAGE_SCHEMA: [&str; 2] = [
    "CREATE TABLE IF NOT EXISTS request_usage (request_id TEXT PRIMARY KEY, actor TEXT, payload TEXT, tokens INTEGER, unknown INTEGER, attempts INTEGER, input_tokens INTEGER, output_tokens INTEGER, cache_read_tokens INTEGER, cache_write_tokens INTEGER)",
    "CREATE TABLE IF NOT EXISTS usage_budget (id INTEGER PRIMARY KEY CHECK(id=1), payload TEXT)",
];

/// `usage_report`'s `aggregates`.
const AGGREGATES: &str = "count(*) requests, coalesce(sum(tokens),0) observed_tokens, coalesce(sum(unknown),0) unknown_usage_requests, coalesce(sum(attempts),0) attempts, coalesce(sum(input_tokens),0) reported_input_tokens, coalesce(sum(output_tokens),0) reported_output_tokens, coalesce(sum(cache_read_tokens),0) reported_cache_read_tokens, coalesce(sum(cache_write_tokens),0) reported_cache_write_tokens, count(cache_read_tokens) cache_read_known_requests, count(cache_write_tokens) cache_write_known_requests";

/// The two totals of [`AGGREGATES`] the budget and the receipt read
/// (#2340), by the same expressions, so each is the report's value and
/// type; `repository_usage_tests.rs` pins that they are its own.
const STANDING: &str =
    "coalesce(sum(tokens),0) observed_tokens, coalesce(sum(unknown),0) unknown_usage_requests";

impl BoardUsage for SqliteBoard<'_> {
    fn usage_report(&self) -> Result<UsageReport, BoardError> {
        self.usage_schema()?;
        let budget = self.stored_budget()?;
        let totals = self
            .connection
            .query_row(
                &format!("SELECT {AGGREGATES} FROM request_usage"),
                [],
                usage_row,
            )
            .map_err(failed)?;
        let members = self.usage_rows(&format!(
            "SELECT actor member, {AGGREGATES} FROM request_usage GROUP BY actor ORDER BY actor"
        ))?;
        let mut statement = self
            .connection
            .prepare("SELECT actor,payload FROM request_usage ORDER BY rowid DESC LIMIT 10")
            .map_err(failed)?;
        let recent_requests = statement
            .query_map([], |row| {
                fetched(row)?;
                Ok(RecentRequest {
                    member: cell_at(row, 0)?,
                    observation: loaded(row, 1)?,
                })
            })
            .map_err(failed)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(failed)?;
        debug_assert!(recent_requests.len() <= 10, "the ten latest at most");
        Ok(UsageReport {
            budget,
            totals,
            members,
            recent_requests,
        })
    }

    fn member_usage(&self) -> Result<Vec<UsageRow>, BoardError> {
        let exists = self.count(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='request_usage'",
        )?;
        match exists {
            0 => Ok(Vec::new()),
            _ => self.usage_rows(&format!(
                "SELECT actor member, {AGGREGATES} FROM request_usage GROUP BY actor ORDER BY actor"
            )),
        }
    }

    fn usage_standing(&self) -> Result<UsageStanding, BoardError> {
        self.usage_schema()?;
        let budget = self.stored_budget()?;
        let (observed_tokens, unknown_usage_requests) = match self.ledger_sums()? {
            Some(sums) => (
                Value::from(sums.observed_tokens),
                Value::from(sums.unknown_usage_requests),
            ),
            None => self
                .connection
                .query_row(
                    &format!("SELECT {STANDING} FROM request_usage"),
                    [],
                    |row| {
                        fetched(row)?;
                        Ok((cell_at(row, 0)?, cell_at(row, 1)?))
                    },
                )
                .map_err(failed)?,
        };
        Ok(UsageStanding {
            budget,
            observed_tokens,
            unknown_usage_requests,
        })
    }

    fn usage_budget(&self) -> Result<Value, BoardError> {
        self.usage_schema()?;
        self.stored_budget()
    }

    fn configure_usage_budget(&self, budget: &Value) -> Result<(), BoardError> {
        debug_assert!(budget.is_object(), "a budget is an object: {budget}");
        self.usage_schema()?;
        let payload = PyJson::try_from(budget)
            .and_then(|budget| py_json::dumps(&budget))
            .map_err(|error| BoardError::new(RefusalKind::Invalid, error.to_string()))?;
        let changed = self
            .connection
            .execute(
                "INSERT INTO usage_budget VALUES(1,?) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload",
                [payload],
            )
            .map_err(failed)?;
        debug_assert_eq!(changed, 1, "the one budget row");
        Ok(())
    }

    fn request_usage(&self, request_id: &str) -> Result<Option<StoredRequestUsage>, BoardError> {
        self.usage_schema()?;
        self.connection
            .query_row(
                "SELECT actor,payload FROM request_usage WHERE request_id=?",
                [request_id],
                |row| {
                    fetched(row)?;
                    Ok(StoredRequestUsage {
                        actor: cell_at(row, 0)?,
                        payload: loaded(row, 1)?,
                    })
                },
            )
            .optional()
            .map_err(failed)
    }

    fn insert_request_usage(&self, usage: &NewRequestUsage) -> Result<(), BoardError> {
        self.usage_schema()?;
        let count = |value: u64| {
            i64::try_from(value).map_err(|_| {
                BoardError::new(RefusalKind::Invalid, format!("count beyond i64: {value}"))
            })
        };
        let reported = |value: Option<u64>| value.map(count).transpose();
        let changed = self
            .connection
            .execute(
                "INSERT INTO request_usage VALUES(?,?,?,?,?,?,?,?,?,?)",
                params![
                    usage.request_id,
                    usage.actor,
                    usage.payload,
                    count(usage.tokens)?,
                    count(usage.unknown)?,
                    count(usage.attempts)?,
                    reported(usage.input_tokens)?,
                    reported(usage.output_tokens)?,
                    reported(usage.cache_read_tokens)?,
                    reported(usage.cache_write_tokens)?,
                ],
            )
            .map_err(failed)?;
        debug_assert_eq!(changed, 1, "one request row");
        Ok(())
    }

    fn update_request_usage(&self, request_id: &str, payload: &str) -> Result<(), BoardError> {
        self.usage_schema()?;
        let changed = self
            .connection
            .execute(
                "UPDATE request_usage SET payload=? WHERE request_id=?",
                params![payload, request_id],
            )
            .map_err(failed)?;
        debug_assert_eq!(changed, 1, "the request row read before it");
        Ok(())
    }

    fn request_usage_count(&self) -> Result<i64, BoardError> {
        self.usage_schema()?;
        match self.ledger_sums()? {
            Some(sums) => Ok(sums.rows),
            None => self.count("SELECT count(*) FROM request_usage"),
        }
    }
}

impl SqliteBoard<'_> {
    /// The ledger's sums now (#2340, [`usage_sums`](super::usage_sums)):
    /// this transaction's own, else the process's kept ones, extended by
    /// the rows after them; `None` when the SQL must sum instead.
    fn ledger_sums(&self) -> Result<Option<UsageSums>, BoardError> {
        let known = self
            .staged_sums
            .borrow_mut()
            .take()
            .or_else(|| self.kept_sums.kept());
        let sums = usage_sums::summed(self.connection, known).map_err(failed)?;
        self.staged_sums.replace(sums.clone());
        Ok(sums)
    }

    /// `Transaction._usage_schema`, run once per transaction: its
    /// `CREATE TABLE IF NOT EXISTS` statements change nothing after the
    /// first run inside it.
    fn usage_schema(&self) -> Result<(), BoardError> {
        if self.usage_schema_created.get() {
            return Ok(());
        }
        for statement in USAGE_SCHEMA {
            self.connection.execute(statement, []).map_err(failed)?;
        }
        self.usage_schema_created.set(true);
        Ok(())
    }

    /// The budget row's payload as `json.loads` reads it, or the default
    /// budget without a row (none is written).
    fn stored_budget(&self) -> Result<Value, BoardError> {
        Ok(self
            .connection
            .query_row("SELECT payload FROM usage_budget WHERE id=1", [], |row| {
                fetched(row)?;
                loaded(row, 0)
            })
            .optional()
            .map_err(failed)?
            .unwrap_or_else(
                || json!({"token_limit": null, "strict_unknown": false, "warned": false}),
            ))
    }

    fn usage_rows(&self, sql: &str) -> Result<Vec<UsageRow>, BoardError> {
        let mut statement = self.connection.prepare(sql).map_err(failed)?;
        statement
            .query_map([], usage_row)
            .map_err(failed)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(failed)
    }
}

/// `dict(row)` of an aggregate row: every column, in order, as stored.
fn usage_row(row: &Row<'_>) -> rusqlite::Result<UsageRow> {
    fetched(row)?;
    let columns = (0..row.as_ref().column_count())
        .map(|index| {
            Ok((
                row.as_ref().column_name(index)?.to_owned(),
                cell_at(row, index)?,
            ))
        })
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(UsageRow { columns })
}

#[cfg(test)]
#[path = "repository_usage_tests.rs"]
mod tests;
