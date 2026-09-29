//! The run-wide section of a run summary (#2313 review M1): what every
//! member of the run did, as the board holds it when the run settles.
//!
//! A harness's [`super::RunSummaryFold`] sees only its own board calls
//! (each member is a process of its own, with a board of its own), so its
//! counts are that process's. The run's own totals are read from the board
//! file every member shares, once, by the coordinator's harness at settle:
//! the tasks by state, the messages by what became of them, each member's
//! recorded requests and their token usage, and the run's wall time from
//! its creation. Counts and kinds only: a member appears as its redacted
//! `actor_ref`, and no board text is kept.
use serde::{Deserialize, Serialize};

use super::run_summary::REQUEST_MEMBERS;
use crate::domain::redaction::Redacted;

/// The run's tasks by the state the board holds them in, as the run's
/// `summary` counts them (a `ready` task waiting on a dependency counts as
/// `blocked`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskStates {
    pub total: u64,
    pub ready: u64,
    pub claimed: u64,
    pub blocked: u64,
    pub submitted: u64,
    pub completed: u64,
}

/// The run's messages: every one sent, and those consumed by their
/// recipient (`acked`) or withdrawn by their sender.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageTotals {
    pub sent: u64,
    pub acked: u64,
    pub withdrawn: u64,
}

/// One member's provider requests the board's ledger holds, and the token
/// usage they reported.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberUsage {
    pub actor_ref: Redacted,
    pub requests: u64,
    /// The tokens the budget counted (`observed_tokens`).
    pub tokens: u64,
    /// Requests whose usage the provider did not report.
    pub unknown_usage_requests: u64,
    pub attempts: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
}

/// The run-wide section of a `swarm_run_summary`: the board's totals for
/// the whole run, every member's work included.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunTotals {
    pub tasks: TaskStates,
    pub messages: MessageTotals,
    /// Each member's requests, in the board's order (by member id), at most
    /// [`REQUEST_MEMBERS`].
    pub usage: Vec<MemberUsage>,
    /// Members past [`REQUEST_MEMBERS`], in no `usage` entry; left out
    /// when none.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub unlisted_usage: u64,
    /// From the run's creation (its `created` event) to the board's read
    /// at settle, on the board's clock; `None` when the board holds no
    /// creation time (a run created before events were kept, or a record
    /// edited from outside).
    pub wall_time_us: Option<u64>,
}

fn is_zero(count: &u64) -> bool {
    *count == 0
}

impl RunTotals {
    /// The totals the board holds: `usage` bounded to [`REQUEST_MEMBERS`]
    /// (the rest counted), and the wall time from `created_at` to
    /// `read_at`, both Unix seconds on the board's clock (never negative;
    /// `None` without a finite creation time).
    pub fn new(
        tasks: TaskStates,
        messages: MessageTotals,
        mut usage: Vec<MemberUsage>,
        created_at: Option<f64>,
        read_at: f64,
    ) -> Self {
        let unlisted = usage.len().saturating_sub(REQUEST_MEMBERS);
        usage.truncate(REQUEST_MEMBERS);
        debug_assert!(usage.len() <= REQUEST_MEMBERS, "the usage is bounded");
        Self {
            tasks,
            messages,
            usage,
            unlisted_usage: u64::try_from(unlisted).unwrap_or(u64::MAX),
            wall_time_us: created_at
                .filter(|created| created.is_finite() && read_at.is_finite())
                .map(|created| microseconds(read_at - created)),
        }
    }
}

/// `seconds` as whole microseconds, `0` for a negative span.
fn microseconds(seconds: f64) -> u64 {
    let micros = (seconds * 1_000_000.0).round();
    match micros > 0.0 {
        // Saturating: a float past u64's range converts to u64::MAX.
        true => micros as u64,
        false => 0,
    }
}

#[cfg(test)]
#[path = "run_totals_tests.rs"]
mod tests;
