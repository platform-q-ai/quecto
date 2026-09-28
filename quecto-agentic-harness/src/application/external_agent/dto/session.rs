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
    /// Every process's cumulative tokens (`modelUsage`), summed.
    pub tokens: TokenCounts,
    /// The session's cost at list price: the sum of every turn's charge,
    /// micro-USD.
    pub cost_micro_usd: u64,
    pub turns: usize,
    /// Every permission denial reported, including those the bounded
    /// audit no longer holds.
    pub guardrail_denials: usize,
    /// Every rate-limit event that warranted a warning.
    pub admission_warnings: usize,
    /// Every event of a type the vocabulary does not know.
    pub unknown_events: usize,
    /// Every line of the stream the adapter skipped unread (#2286).
    pub skipped_lines: usize,
}
