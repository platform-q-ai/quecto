//! A session's usage as the inspect-session-usage use case reports it
//! (#1855): billed token totals, their cost and the request diagnostics,
//! accumulated across the session's turns, and the report a caller
//! presents for `get_session_stats`.
use crate::domain::inference::events::request_observation::RequestDiagnostics;

/// Billed tokens by kind; `total` is input plus output.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenTotals {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub total: u64,
}

/// The usage a session has retained across its turns.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionUsage {
    pub request_diagnostics: RequestDiagnostics,
    pub tokens: TokenTotals,
    /// What the priced replies cost.
    pub cost_micro_usd: u64,
    /// Replies that billed tokens with no price to cost them at (#1855).
    pub unpriced_requests: u64,
}

impl SessionUsage {
    /// The cost in USD, for display.
    pub fn cost_usd(&self) -> f64 {
        self.cost_micro_usd as f64 / 1_000_000.0
    }

    /// The cost, if any of it is known (#1855 review L1): `None` when the
    /// session billed usage and none of it was priced, so unpriced never
    /// reads as free. A session that used nothing cost zero.
    pub fn known_cost_micro_usd(&self) -> Option<u64> {
        match (self.cost_micro_usd, self.unpriced_requests) {
            (0, unpriced) if unpriced > 0 => None,
            (cost, _) => Some(cost),
        }
    }

    /// The share of input served from the cache, when any input was billed.
    pub fn cache_hit_ratio(&self) -> Option<f64> {
        crate::domain::inference::services::usage_accounting::cache_hit_ratio(
            self.tokens.input,
            self.tokens.cache_read,
            self.tokens.cache_write,
        )
    }
}

/// How many messages of each kind a session's history holds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MessageCounts {
    pub user: usize,
    pub assistant: usize,
    pub tool_calls: usize,
    pub tool_results: usize,
    pub total: usize,
}

/// A session's usage report: its history, its retained usage and its
/// context occupancy against the active model's window (`0` = unknown).
#[derive(Debug, Clone, PartialEq)]
pub struct SessionUsageReport {
    pub session_key: String,
    pub messages: MessageCounts,
    pub usage: SessionUsage,
    /// The known cost ([`SessionUsage::known_cost_micro_usd`]).
    pub cost_micro_usd: Option<u64>,
    pub cost_usd: f64,
    pub cache_hit_ratio: Option<f64>,
    pub context_tokens: usize,
    pub max_context_tokens: usize,
}
