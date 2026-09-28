//! How an external agent's turn ended, and what it used (#2285).
//!
//! The traps spike #2264 found live here: a `result` whose `subtype` says
//! `success` can still be an error, so a turn is [`TurnEnd::Completed`]
//! only on `terminal_reason == "completed"` with `is_error == false`; and
//! `result.usage` is per turn while `total_cost_usd` is cumulative, so a
//! turn's cost is the delta of the cumulative total, kept in whole
//! micro-USD and rounded once per result.

use super::stream::{ResultEvent, TokenCounts};

/// The `terminal_reason` of a turn that finished its work.
pub const TERMINAL_REASON_COMPLETED: &str = "completed";

/// How one turn ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnEnd {
    Completed,
    Failed(TurnFailure),
}

/// Why a turn did not complete: every signal the stream gave.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TurnFailure {
    pub terminal_reason: Option<String>,
    pub api_error_status: Option<u16>,
    /// The `error` an assistant event of the turn carried
    /// (`authentication_failed` …).
    pub assistant_error: Option<String>,
}

impl TurnEnd {
    /// Classify a turn from its `result` and any assistant error the turn
    /// carried. Completed only when the stream affirms both halves:
    /// `terminal_reason` is `completed` and `is_error` is `false`. A
    /// missing field, or an assistant error, is a failure. `subtype` is
    /// never consulted.
    pub fn classify(result: &ResultEvent, assistant_error: Option<&str>) -> Self {
        // RED (#2285): not implemented yet.
        let _ = (result, assistant_error);
        Self::Completed
    }

    pub fn is_completed(&self) -> bool {
        matches!(self, Self::Completed)
    }
}

/// What one turn used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TurnUsage {
    /// This turn's tokens (`result.usage`).
    pub tokens: TokenCounts,
    /// This turn's cost: the cumulative total's delta, micro-USD.
    pub cost_micro_usd: u64,
    /// The process's cumulative cost after this turn, micro-USD.
    pub total_cost_micro_usd: u64,
}

/// Turns a stream of cumulative totals into per-turn usage.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageLedger {
    total_cost_micro_usd: u64,
    cumulative_tokens: TokenCounts,
}

impl UsageLedger {
    /// Record one turn's `result`. The turn's cost is the new cumulative
    /// total (rounded once, to micro-USD) less the previous one; a result
    /// without a usable total leaves the total as it was.
    pub fn record(&mut self, result: &ResultEvent) -> TurnUsage {
        // RED (#2285): not implemented yet.
        let total = result.total_cost_usd.and_then(micro_usd).unwrap_or(0);
        self.total_cost_micro_usd = total;
        TurnUsage {
            tokens: result.usage,
            cost_micro_usd: total,
            total_cost_micro_usd: total,
        }
    }

    /// The process's cumulative cost, micro-USD.
    pub fn total_cost_micro_usd(&self) -> u64 {
        self.total_cost_micro_usd
    }

    /// The process's cumulative tokens (`modelUsage`, summed over models).
    pub fn cumulative_tokens(&self) -> TokenCounts {
        self.cumulative_tokens
    }
}

/// `usd` in whole micro-USD, rounded once; `None` unless it is a finite,
/// non-negative amount that fits.
pub fn micro_usd(usd: f64) -> Option<u64> {
    // RED (#2285): not implemented yet.
    Some(usd as u64)
}

#[cfg(test)]
#[path = "turn_tests.rs"]
mod tests;
