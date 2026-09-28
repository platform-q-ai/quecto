//! Usage-budget policy ported from `swarm_policy.py` (#2267): the budget
//! decision and the validation of one request's usage record.
use serde_json::Value;

use super::BoardError;

/// A run's token budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsageBudget {
    pub token_limit: Option<u64>,
    pub strict_unknown: bool,
    pub warned: bool,
}

/// A run's accumulated usage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UsageTotals {
    pub observed_tokens: u64,
    pub unknown_usage_requests: u64,
}

/// What the budget allows next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsageDecision {
    Allow,
    Warn,
    Pause,
}

impl UsageDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Warn => "warn",
            Self::Pause => "pause",
        }
    }
}

/// The fields a request record may measure, each null or a count.
pub const MEASURED_FIELDS: [&str; 5] = [
    "input_tokens",
    "context_input_tokens",
    "output_tokens",
    "cache_read_tokens",
    "cache_write_tokens",
];
/// How a request ended.
pub const REQUEST_OUTCOMES: [&str; 4] = ["succeeded", "failed", "cancelled", "rejected"];
/// The largest count a request record may carry: `2**32 - 1`.
pub const MAX_REQUEST_COUNT: u64 = (1 << 32) - 1;
/// The most characters (code points, as Python counts) in a request id.
pub const MAX_REQUEST_ID_CHARS: usize = 128;

/// No limit allows; a strict budget pauses on any unmeasured request; the
/// limit pauses; 80% of it warns (integer math, as Python's unbounded ints).
pub fn usage_budget_decision(budget: &UsageBudget, totals: &UsageTotals) -> UsageDecision {
    let Some(limit) = budget.token_limit else {
        return UsageDecision::Allow;
    };
    if budget.strict_unknown && totals.unknown_usage_requests > 0 {
        return UsageDecision::Pause;
    }
    let (observed, limit) = (u128::from(totals.observed_tokens), u128::from(limit));
    if observed >= limit {
        UsageDecision::Pause
    } else if observed * 5 >= limit * 4 {
        UsageDecision::Warn
    } else {
        UsageDecision::Allow
    }
}

/// One request's measurement: `(tokens, unknown, attempts)`. `tokens` is the
/// context input plus output when both are measured, else 0; `unknown` is 1
/// only for an answered (succeeded or failed) request with attempts whose
/// usage is not measured, since a cancelled or rejected attempt never had
/// usage to report. The record is an object with a request id of 1 through
/// 128 characters, each measured field null or an integer (never a boolean or
/// a float) of 0 through `2**32 - 1`, an attempt count of the same range and a
/// known outcome.
pub fn request_measurement(record: &Value) -> Result<(u64, u64, u64), BoardError> {
    let observation = || BoardError::new("invalid request observation");
    let fields = match record.as_object() {
        Some(fields) if identified(fields.get("request_id")) => fields,
        _ => return Err(observation()),
    };
    let mut counts = [None; MEASURED_FIELDS.len()];
    for (slot, field) in counts.iter_mut().zip(MEASURED_FIELDS) {
        *slot = match fields.get(field) {
            None | Some(Value::Null) => None,
            Some(value) => Some(
                count(value)
                    .ok_or_else(|| BoardError::new(format!("invalid request usage {field}")))?,
            ),
        };
    }
    let attempts = fields.get("instrumented_attempts").and_then(count);
    let outcome = fields.get("outcome").and_then(Value::as_str);
    let (Some(attempts), Some(outcome @ ("succeeded" | "failed" | "cancelled" | "rejected"))) =
        (attempts, outcome)
    else {
        return Err(observation());
    };
    debug_assert!(REQUEST_OUTCOMES.contains(&outcome), "a known outcome");
    let [_, context, output, _, _] = counts;
    let answered = matches!(outcome, "succeeded" | "failed");
    let (tokens, known) = match (context, output) {
        (Some(context), Some(output)) => (context + output, true),
        _ => (0, false),
    };
    debug_assert!(tokens <= 2 * MAX_REQUEST_COUNT, "two bounded counts");
    Ok((
        tokens,
        u64::from(answered && attempts > 0 && !known),
        attempts,
    ))
}

/// A request id of 1 through 128 characters.
fn identified(request_id: Option<&Value>) -> bool {
    request_id
        .and_then(Value::as_str)
        .is_some_and(|id| (1..=MAX_REQUEST_ID_CHARS).contains(&id.chars().count()))
}

/// A JSON integer of 0 through `2**32 - 1` (`type(value) is int`: never a
/// boolean or a float).
fn count(value: &Value) -> Option<u64> {
    value.as_u64().filter(|count| *count <= MAX_REQUEST_COUNT)
}

#[cfg(test)]
#[path = "usage_tests.rs"]
mod usage_tests;
