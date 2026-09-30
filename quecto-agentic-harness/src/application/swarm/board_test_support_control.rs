//! Test support (compiled only under `cfg(test)`): the in-memory board's
//! usage report, budget and request ledger (#2273, #2274) and loss scan.
//! The SQLite adapter's contract tests pin the real SQL.
use serde_json::{Value, json};

use super::{BoardState, MemoryTransaction};
use crate::application::swarm::dto::{
    NewRequestUsage, StoredRequestUsage, UsageReport, UsageRow, UsageStanding,
};
use crate::application::swarm::ports::BoardUsage;
use crate::domain::swarm::{BoardError, RunState};

/// One recorded `events` row.
#[derive(Clone, Debug, PartialEq)]
pub struct RecordedEvent {
    pub actor: String,
    pub time: f64,
    pub action: String,
    pub detail: Value,
}

/// The ledger reads a board served (#2340): each `usage_report` reads the
/// whole ledger (both aggregates and the latest observations), each
/// `usage_standing` one aggregate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UsageReads {
    pub whole_reports: usize,
    pub standings: usize,
}

impl BoardUsage for MemoryTransaction<'_> {
    fn usage_report(&self) -> Result<UsageReport, BoardError> {
        self.state.borrow_mut().usage_reads.whole_reports += 1;
        Ok(self.stored_usage())
    }

    fn usage_standing(&self) -> Result<UsageStanding, BoardError> {
        self.state.borrow_mut().usage_reads.standings += 1;
        let report = self.stored_usage();
        let total = |key: &str| report.totals.get(key).cloned().unwrap_or(Value::Null);
        Ok(UsageStanding {
            observed_tokens: total("observed_tokens"),
            unknown_usage_requests: total("unknown_usage_requests"),
            budget: report.budget,
        })
    }

    fn member_usage(&self) -> Result<Vec<UsageRow>, BoardError> {
        let state = self.state.borrow();
        Ok(state
            .usage
            .as_ref()
            .map(|usage| usage.members.clone())
            .unwrap_or_default())
    }

    fn usage_budget(&self) -> Result<Value, BoardError> {
        Ok(self.stored_usage().budget)
    }

    fn configure_usage_budget(&self, budget: &Value) -> Result<(), BoardError> {
        self.note(format!("configure_usage_budget {budget}"));
        let mut state = self.state.borrow_mut();
        state.usage.get_or_insert_with(default_usage).budget = budget.clone();
        Ok(())
    }

    fn request_usage(&self, request_id: &str) -> Result<Option<StoredRequestUsage>, BoardError> {
        Ok(self
            .state
            .borrow()
            .request_usage
            .iter()
            .find(|row| row.request_id == request_id)
            .map(|row| StoredRequestUsage {
                actor: Value::from(row.actor.as_str()),
                payload: serde_json::from_str(&row.payload)
                    .expect("the fake stores the encoder's JSON text"),
            }))
    }

    fn insert_request_usage(&self, row: &NewRequestUsage) -> Result<(), BoardError> {
        self.note(format!("insert_request_usage {}", row.request_id));
        let mut state = self.state.borrow_mut();
        let report = state.usage.get_or_insert_with(default_usage);
        for (column, added) in [
            ("requests", 1),
            ("observed_tokens", row.tokens),
            ("unknown_usage_requests", row.unknown),
        ] {
            if let Some((_, total)) = report.totals.columns.iter_mut().find(|(n, _)| n == column) {
                *total = json!(total.as_u64().unwrap_or(0) + added);
            }
        }
        state.request_usage.push(row.clone());
        Ok(())
    }

    fn update_request_usage(&self, request_id: &str, payload: &str) -> Result<(), BoardError> {
        self.note(format!("update_request_usage {request_id}"));
        let mut state = self.state.borrow_mut();
        let row = state
            .request_usage
            .iter_mut()
            .find(|row| row.request_id == request_id)
            .expect("the updated request was read first");
        payload.clone_into(&mut row.payload);
        Ok(())
    }

    fn request_usage_count(&self) -> Result<i64, BoardError> {
        let state = self.state.borrow();
        let listed = state
            .usage
            .as_ref()
            .and_then(|report| report.totals.get("requests").and_then(Value::as_i64));
        Ok(listed.unwrap_or(0))
    }
}

impl MemoryTransaction<'_> {
    /// The report as stored, read without counting a read.
    fn stored_usage(&self) -> UsageReport {
        self.state
            .borrow()
            .usage
            .clone()
            .unwrap_or_else(default_usage)
    }
}

/// The report of a board with no budget row and no requests.
fn default_usage() -> UsageReport {
    usage(
        json!({"token_limit": null, "strict_unknown": false, "warned": false}),
        0,
        0,
    )
}

/// A report with `budget` and totals of `observed` tokens and `unknown`
/// unmeasured requests, no member rows and no recent requests.
pub fn usage(budget: Value, observed: u64, unknown: u64) -> UsageReport {
    UsageReport {
        budget,
        totals: UsageRow {
            columns: vec![
                ("requests".to_owned(), json!(0)),
                ("observed_tokens".to_owned(), json!(observed)),
                ("unknown_usage_requests".to_owned(), json!(unknown)),
            ],
        },
        members: Vec::new(),
        recent_requests: Vec::new(),
    }
}

/// `lost_members` over the recorded events: the latest `scope_unknown`
/// newer than the latest `activated`, by event position.
pub(super) fn lost_members(events: &[RecordedEvent], members: &[&str]) -> Vec<String> {
    let latest = |member: &str, action: &str| {
        events
            .iter()
            .rposition(|event| {
                event.action == action
                    && event.detail.get("member").and_then(Value::as_str) == Some(member)
            })
            .map(|index| index + 1)
    };
    members
        .iter()
        .filter(|member| latest(member, "scope_unknown") > latest(member, "activated"))
        .map(|member| (*member).to_owned())
        .collect()
}

/// `state` paused at `started` holding `outcome` (for `reason`), as the
/// board records a pause: the run's status and a `paused` event.
pub fn paused(mut state: BoardState, started: f64, outcome: Option<(&str, &str)>) -> BoardState {
    let run = &mut state.run.as_mut().expect("a run to pause").record;
    run.status = Some(RunState::PAUSED);
    run.outcome = outcome.map(|(outcome, _)| outcome.to_owned());
    run.outcome_reason = outcome.map(|(_, reason)| reason.to_owned());
    state.events.push(recorded(
        "parent",
        "paused",
        json!({"reason": "hold", "started": started}),
    ));
    state
}

/// An event `actor` recorded at time 0.
pub fn recorded(actor: &str, action: &str, detail: Value) -> RecordedEvent {
    RecordedEvent {
        actor: actor.to_owned(),
        time: 0.0,
        action: action.to_owned(),
        detail,
    }
}
