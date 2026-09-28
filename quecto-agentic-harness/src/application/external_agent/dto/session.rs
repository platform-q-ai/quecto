//! The member's session statistics (#2285).

use crate::domain::external_agent::stream::TokenCounts;

/// The member's session statistics.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionTotals {
    pub session_key: Option<String>,
    pub model: Option<String>,
    pub user_messages: usize,
    pub assistant_messages: usize,
    pub tool_calls: usize,
    pub tool_results: usize,
    /// Cumulative tokens (`modelUsage`).
    pub tokens: TokenCounts,
    /// Cumulative cost at list price, micro-USD.
    pub cost_micro_usd: u64,
    pub turns: usize,
}
