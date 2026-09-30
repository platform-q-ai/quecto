//! The request ledger's sums a process keeps between transactions
//! (#2340), so the budget's standing and the ledger's row count cost the
//! same at the end of a long run as at its start.
//!
//! Every transaction opens a fresh connection, so an aggregate over the
//! ledger reads each of its pages from the file again: a recorded request
//! and its control receipt cost about half a microsecond per row already
//! recorded, growing with the run. The ledger only grows (the board inserts
//! rows and rewrites a row's payload, never its counts, and deletes none),
//! so the sums up to a row stay the sums up to that row. A process keeps
//! them, as of the last row it summed, and a later transaction adds only
//! the rows after it, in rowid order, which is the order SQLite's `sum`
//! adds them in: an integer overflow is met at the same row, and the
//! caller then runs the SQL, which raises it.
//!
//! What is kept is checked before it is used: the run is the same (a
//! board recreated at the same path, or a new run, sums afresh) and the
//! last row summed still holds the request id it held. The rows after it
//! must hold integer (or NULL) counts and a text request id; any other
//! value, which only an edit from outside the board writes, is summed by
//! the SQL instead, as stored. Sums are published only after the
//! transaction that computed them commits, so a rolled-back insert is never
//! counted.
//!
//! Run ids and request ids are assumed unique: the board generates run
//! ids and the harness request ids as UUIDv4s, so a recreated board or a
//! new run cannot present the same run id with the same request id at the
//! same rowid. Restoring an older copy of a board file under a running
//! process is an edit from outside the board, below.
//!
//! Permitted divergence (`outside_edited_control_records`): an edit from
//! outside the board to a row already summed (its counts changed, or it
//! or an earlier row deleted) is not seen by a process that kept its sums;
//! Python, which sums on every read, sees it.
use std::sync::{Arc, Mutex, PoisonError};

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OptionalExtension, params};

/// The ledger's sums, as of its row `last` (`None`: an empty ledger).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct UsageSums {
    /// The run whose board they were summed on.
    run_id: String,
    /// The last row summed: its rowid and its request id.
    last: Option<(i64, String)>,
    /// `count(*)`.
    pub(super) rows: i64,
    /// `sum(tokens)`, 0 for none.
    pub(super) observed_tokens: i64,
    /// `sum(unknown)`, 0 for none.
    pub(super) unknown_usage_requests: i64,
}

/// The sums a process's transactions on one board file share: those of
/// the latest committed transaction that computed any.
#[derive(Clone, Debug, Default)]
pub(super) struct KeptSums {
    sums: Arc<Mutex<Option<UsageSums>>>,
    #[cfg(test)]
    work: Arc<Mutex<LedgerWork>>,
}

impl KeptSums {
    /// The ledger work done so far (tests only).
    #[cfg(test)]
    pub(crate) fn work(&self) -> LedgerWork {
        *self.work.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Notes a row the kept sums were extended by (tests only; a no-op
    /// otherwise).
    fn noted_row(&self) {
        #[cfg(test)]
        {
            self.work
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .rows_scanned += 1;
        }
    }

    /// Notes a whole-ledger SQL read: a usage report, or the sums'
    /// fallback (tests only; a no-op otherwise).
    pub(super) fn noted_whole_ledger_read(&self) {
        #[cfg(test)]
        {
            self.work
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .whole_ledger_reads += 1;
        }
    }

    /// The sums kept, also after a panic elsewhere poisoned the lock:
    /// they are replaced whole, never left half-written.
    pub(super) fn kept(&self) -> Option<UsageSums> {
        self.sums
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Keeps `sums`, which a committed transaction computed.
    pub(super) fn keep(&self, sums: UsageSums) {
        *self.sums.lock().unwrap_or_else(PoisonError::into_inner) = Some(sums);
    }
}

/// The ledger's sums now, from `known` when it is still this board's (the
/// rows after its last added), or from the first row; `None` when a value
/// only an edit writes means the SQL must sum instead (or the board holds
/// no run to key them by).
///
/// # Errors
/// The store's.
pub(super) fn summed(
    connection: &Connection,
    kept: &KeptSums,
    known: Option<UsageSums>,
) -> rusqlite::Result<Option<UsageSums>> {
    let Some(run_id) = run_id(connection)? else {
        return Ok(None);
    };
    let base = match known {
        Some(known) if known.run_id == run_id && still_last(connection, known.last.as_ref())? => {
            known
        }
        Some(_) | None => UsageSums {
            run_id,
            last: None,
            rows: 0,
            observed_tokens: 0,
            unknown_usage_requests: 0,
        },
    };
    extended(connection, kept, base)
}

/// The run row's id, when it is UTF-8 text.
fn run_id(connection: &Connection) -> rusqlite::Result<Option<String>> {
    connection
        .query_row("SELECT id FROM run", [], |row| {
            Ok(match row.get_ref(0)? {
                ValueRef::Text(bytes) => std::str::from_utf8(bytes).ok().map(str::to_owned),
                ValueRef::Null | ValueRef::Integer(_) | ValueRef::Real(_) | ValueRef::Blob(_) => {
                    None
                }
            })
        })
        .optional()
        .map(Option::flatten)
}

/// Whether the last row summed still holds its request id (an empty
/// ledger's sums have no row to check).
fn still_last(connection: &Connection, last: Option<&(i64, String)>) -> rusqlite::Result<bool> {
    let Some((rowid, request_id)) = last else {
        return Ok(true);
    };
    let held = connection
        .query_row(
            "SELECT request_id FROM request_usage WHERE rowid=?",
            [rowid],
            |row| {
                Ok(matches!(row.get_ref(0)?, ValueRef::Text(held) if held == request_id.as_bytes()))
            },
        )
        .optional()?;
    Ok(held == Some(true))
}

/// `sums` with every row after its last added, in rowid order.
fn extended(
    connection: &Connection,
    kept: &KeptSums,
    mut sums: UsageSums,
) -> rusqlite::Result<Option<UsageSums>> {
    let mut statement = match &sums.last {
        Some(_) => connection.prepare(
            "SELECT rowid, request_id, tokens, unknown FROM request_usage WHERE rowid>? ORDER BY rowid",
        )?,
        None => connection.prepare(
            "SELECT rowid, request_id, tokens, unknown FROM request_usage ORDER BY rowid",
        )?,
    };
    let mut rows = match &sums.last {
        Some((rowid, _)) => statement.query(params![rowid])?,
        None => statement.query([])?,
    };
    while let Some(row) = rows.next()? {
        let rowid: i64 = row.get(0)?;
        let (ValueRef::Text(request_id), Some(tokens), Some(unknown)) = (
            row.get_ref(1)?,
            count(row.get_ref(2)?),
            count(row.get_ref(3)?),
        ) else {
            return Ok(None);
        };
        let Ok(request_id) = std::str::from_utf8(request_id) else {
            return Ok(None);
        };
        debug_assert!(
            sums.last.as_ref().is_none_or(|(last, _)| rowid > *last),
            "rows are added in rowid order"
        );
        let (Some(rows), Some(observed), Some(unknown)) = (
            sums.rows.checked_add(1),
            sums.observed_tokens.checked_add(tokens),
            sums.unknown_usage_requests.checked_add(unknown),
        ) else {
            // SQLite's `sum` overflows at this same row: the SQL raises it.
            return Ok(None);
        };
        kept.noted_row();
        sums.rows = rows;
        sums.observed_tokens = observed;
        sums.unknown_usage_requests = unknown;
        sums.last = Some((rowid, request_id.to_owned()));
    }
    Ok(Some(sums))
}

/// A stored count as `sum` adds it: an integer, or NULL as nothing; any
/// other value is the SQL's to sum.
fn count(value: ValueRef<'_>) -> Option<i64> {
    match value {
        ValueRef::Integer(count) => Some(count),
        ValueRef::Null => Some(0),
        ValueRef::Real(_) | ValueRef::Text(_) | ValueRef::Blob(_) => None,
    }
}

/// The ledger work a repository's board calls did (#2340, tests only):
/// the rows the kept sums were extended by, and the times SQL read the
/// whole ledger (a usage report, or the sums' fallback), so a test asserts
/// the work of a call stays the same however long the ledger. Kept beside
/// the sums, never ambient.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LedgerWork {
    pub(crate) rows_scanned: u64,
    pub(crate) whole_ledger_reads: u64,
}

#[cfg(test)]
#[path = "usage_sums_tests.rs"]
mod tests;
