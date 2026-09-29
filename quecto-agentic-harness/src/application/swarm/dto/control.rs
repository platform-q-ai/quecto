//! Run control and the control receipt (#2273): the requests and answers
//! of `PauseRun`, `ResumeRun`, `ResumeRunExternally`, `CloseRun`,
//! `ExtendRunDeadline`, `StopRun` and `ReadControlStatus`, and the usage
//! report `ReadUsageReport` answers and `BoardUsage` reads.
//!
//! As in [`super::tasks`], `actor` is the member the call acts as and every
//! other argument stays the JSON value the caller passed: Python type-checks
//! a reason, a status and a number of seconds at run time.
use serde_json::{Map, Value};

/// `Workbench.pause(reason)` as `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PauseRunRequest {
    pub actor: String,
    pub reason: Value,
}

/// `Workbench.stop(status, reason)` as `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StopRunRequest {
    pub actor: String,
    pub status: Value,
    pub reason: Value,
}

/// `Workbench._extend_deadline(seconds)` as `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtendRunDeadlineRequest {
    pub actor: String,
    pub seconds: Value,
}

/// Whether a control call changed the run or found it already as asked:
/// `pause` of a paused run, `_resume_external` of a running one, `stop`
/// of a run already holding that outcome or cancelled. A no-op answers
/// the receipt as the change does and records no event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunTransition {
    Applied,
    Unchanged,
}

/// `Coordination._receipt`: the repository's `control_receipt` and the
/// blockers a resume would hit now (#1924). The supervisor's
/// `swarm_control` decodes it field by field (`RunControlReceipt`).
#[derive(Clone, Debug, PartialEq)]
pub struct ControlReceipt {
    /// `run.status` as stored; `None` for NULL.
    pub status: Option<String>,
    /// The outcome a paused run holds (#1729).
    pub outcome: Option<String>,
    /// `run.outcome_reason`.
    pub reason: Option<String>,
    /// The id of the latest `paused` or `resumed` event, `0` for none.
    pub generation: i64,
    /// The usage budget as the report reads it, then `observed_tokens` and
    /// `unknown_usage_requests` from its totals, in Python's dict order.
    pub budget: Map<String, Value>,
    /// Empty unless the run is paused.
    pub resume_blockers: Vec<String>,
}

/// A control call's receipt and whether it changed the run.
#[derive(Clone, Debug, PartialEq)]
pub struct ControlAnswer {
    pub transition: RunTransition,
    pub receipt: ControlReceipt,
}

/// One row of the usage aggregates, keyed by the SQL's column aliases,
/// which Python's `dict(row)` makes the JSON keys: every column in order,
/// as stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageRow {
    pub columns: Vec<(String, Value)>,
}

impl UsageRow {
    /// The value in `column`, when the row has that column.
    pub fn get(&self, column: &str) -> Option<&Value> {
        self.columns
            .iter()
            .find(|(name, _)| name == column)
            .map(|(_, value)| value)
    }
}

/// One of the latest request observations: its actor and its payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentRequest {
    pub member: Value,
    pub observation: Value,
}

/// `Transaction.usage_report()`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageReport {
    /// The stored budget payload as `json.loads` reads it, or
    /// `{token_limit: null, strict_unknown: false, warned: false}` when
    /// there is no budget row (none is written).
    pub budget: Value,
    /// The aggregates over every request.
    pub totals: UsageRow,
    /// The aggregates per actor, `member` first, ordered by actor.
    pub members: Vec<UsageRow>,
    /// The ten latest observations, newest first.
    pub recent_requests: Vec<RecentRequest>,
}
