//! Test support (compiled only under `cfg(test)`): the in-memory board's
//! usage report and loss scan (#2273). The SQLite adapter's contract tests
//! pin the real SQL.
use serde_json::{Value, json};

use super::{BoardState, MemoryTransaction, RecordedEvent};
use crate::application::swarm::dto::{UsageReport, UsageRow};
use crate::application::swarm::ports::BoardUsage;
use crate::domain::swarm::{BoardError, RunState};

impl BoardUsage for MemoryTransaction<'_> {
    fn usage_report(&self) -> Result<UsageReport, BoardError> {
        Ok(self.state.borrow().usage.clone().unwrap_or_else(|| {
            usage(
                json!({"token_limit": null, "strict_unknown": false, "warned": false}),
                0,
                0,
            )
        }))
    }
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
