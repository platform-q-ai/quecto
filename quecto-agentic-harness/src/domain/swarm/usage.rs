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
        "allow"
    }
}

pub fn usage_budget_decision(_budget: &UsageBudget, _totals: &UsageTotals) -> UsageDecision {
    UsageDecision::Allow
}

/// One request's measurement: `(tokens, unknown, attempts)`.
pub fn request_measurement(_record: &Value) -> Result<(u64, u64, u64), BoardError> {
    Err(BoardError::new(""))
}

#[cfg(test)]
#[path = "usage_tests.rs"]
mod usage_tests;
