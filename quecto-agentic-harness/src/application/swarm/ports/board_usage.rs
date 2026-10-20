//! The coordination board's request usage and budgets (#2273, #2274).

use crate::application::swarm::dto::{
    NewRequestUsage, StoredRequestUsage, UsageReport, UsageRow, UsageStanding,
};
use crate::domain::swarm::value_objects::board_error::BoardError;

/// The request-usage ledger and the stored budget, never applied
/// (#2273, #2274). Each method creates `request_usage` and `usage_budget`
/// when absent, as Python's `_usage_schema` does, before it reads them.
pub trait BoardUsage {
    /// `Transaction.usage_report`: `request_usage` and `usage_budget` are
    /// created when absent (Python's statements, in its order), then the
    /// budget, the totals, the per-actor aggregates and the ten latest
    /// observations are read. Without a budget row the budget reads as
    /// `{token_limit: null, strict_unknown: false, warned: false}` and no
    /// row is written.
    fn usage_report(&self) -> Result<UsageReport, BoardError>;
    /// `usage_report()['members']` alone (#2313 final review): the
    /// per-member aggregates, reading no request payload and creating no
    /// table; none when the board holds no `request_usage` table yet.
    fn member_usage(&self) -> Result<Vec<UsageRow>, BoardError>;
    /// The budget and the two totals of [`Self::usage_report`] (#2340),
    /// read as it reads them (the tables created when absent, the default
    /// budget without a row) but with one aggregate over the ledger and
    /// nothing else: what the control receipt needs on every request.
    fn usage_standing(&self) -> Result<UsageStanding, BoardError>;
    /// Makes the usage tables when absent, reading nothing.
    fn usage_tables(&self) -> Result<(), BoardError>;
    /// The row of `request_id`, when the ledger holds one.
    fn request_usage(&self, request_id: &str) -> Result<Option<StoredRequestUsage>, BoardError>;
    /// `INSERT INTO request_usage VALUES(?,?,?,?,?,?,?,?,?,?)`, the
    /// payload stored as the text given.
    fn insert_request_usage(&self, usage: &NewRequestUsage) -> Result<(), BoardError>;
    /// The stored payload of `request_id`, which the caller has read in this
    /// transaction, becomes `payload`, the record's `encode()` text.
    fn update_request_usage(&self, request_id: &str, payload: &str) -> Result<(), BoardError>;
    /// `SELECT count(*) FROM request_usage`.
    fn request_usage_count(&self) -> Result<i64, BoardError>;
}
