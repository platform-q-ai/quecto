//! `BoardUsage` over the SQLite store (#2273): the usage report by
//! Python's SQL (`swarm_repository.Transaction.usage_report`), copied
//! verbatim. The two tables are created lazily, inside the transaction and
//! in Python's order, so a board either implementation read lists them at
//! the same `sqlite_master` positions; the aggregates' column aliases are
//! the report's JSON keys, and `count(cache_read_tokens)` counts the
//! measured requests alone, so the SQL computes them, never Rust.
use rusqlite::{OptionalExtension, Row};
use serde_json::json;

use super::repository::{SqliteBoard, cell_at, failed, fetched};
use super::repository_tasks::loaded;
use crate::application::swarm::dto::{RecentRequest, UsageReport, UsageRow};
use crate::application::swarm::ports::BoardUsage;
use crate::domain::swarm::BoardError;

/// `Transaction._usage_schema`, statement by statement.
const USAGE_SCHEMA: [&str; 2] = [
    "CREATE TABLE IF NOT EXISTS request_usage (request_id TEXT PRIMARY KEY, actor TEXT, payload TEXT, tokens INTEGER, unknown INTEGER, attempts INTEGER, input_tokens INTEGER, output_tokens INTEGER, cache_read_tokens INTEGER, cache_write_tokens INTEGER)",
    "CREATE TABLE IF NOT EXISTS usage_budget (id INTEGER PRIMARY KEY CHECK(id=1), payload TEXT)",
];

/// `usage_report`'s `aggregates`.
const AGGREGATES: &str = "count(*) requests, coalesce(sum(tokens),0) observed_tokens, coalesce(sum(unknown),0) unknown_usage_requests, coalesce(sum(attempts),0) attempts, coalesce(sum(input_tokens),0) reported_input_tokens, coalesce(sum(output_tokens),0) reported_output_tokens, coalesce(sum(cache_read_tokens),0) reported_cache_read_tokens, coalesce(sum(cache_write_tokens),0) reported_cache_write_tokens, count(cache_read_tokens) cache_read_known_requests, count(cache_write_tokens) cache_write_known_requests";

impl BoardUsage for SqliteBoard<'_> {
    fn usage_report(&self) -> Result<UsageReport, BoardError> {
        for statement in USAGE_SCHEMA {
            self.connection.execute(statement, []).map_err(failed)?;
        }
        let budget = self
            .connection
            .query_row("SELECT payload FROM usage_budget WHERE id=1", [], |row| {
                fetched(row)?;
                loaded(row, 0)
            })
            .optional()
            .map_err(failed)?
            .unwrap_or_else(
                || json!({"token_limit": null, "strict_unknown": false, "warned": false}),
            );
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
}

impl SqliteBoard<'_> {
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
