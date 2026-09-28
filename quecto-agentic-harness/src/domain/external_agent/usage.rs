//! What an external agent's turns used (#2285): `result.usage` is per
//! turn while `total_cost_usd` is cumulative for the process, so a turn's
//! cost is the delta of the cumulative total, kept in whole micro-USD and
//! rounded once per result.

use super::stream::{ResultEvent, TokenCounts};

/// What one turn used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TurnUsage {
    /// This turn's tokens (`result.usage`).
    pub tokens: TokenCounts,
    /// This turn's cost: the cumulative total's delta, micro-USD.
    pub cost_micro_usd: u64,
    /// The process's cumulative cost after this turn, micro-USD.
    pub total_cost_micro_usd: u64,
    /// The previous cumulative total, when this result's total was lower:
    /// a new process, charged its whole total.
    pub cumulative_reset_from: Option<u64>,
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
        let previous = self.total_cost_micro_usd;
        let total = result
            .total_cost_usd
            .and_then(micro_usd)
            .unwrap_or(previous);
        debug_assert!(
            total >= previous,
            "a cumulative cost never shrinks: {previous} then {total} micro-USD"
        );
        let cost = total.saturating_sub(previous);
        self.total_cost_micro_usd = total.max(previous);
        let cumulative = result
            .model_usage
            .iter()
            .fold(TokenCounts::default(), |sum, model| sum.plus(model.tokens));
        self.cumulative_tokens = match result.model_usage.is_empty() {
            true => self.cumulative_tokens.plus(result.usage),
            false => cumulative,
        };
        TurnUsage {
            tokens: result.usage,
            cost_micro_usd: cost,
            total_cost_micro_usd: self.total_cost_micro_usd,
            cumulative_reset_from: None,
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
    let micro = (usd * 1_000_000.0).round();
    match micro.is_finite() && micro >= 0.0 && micro <= u64::MAX as f64 {
        // The range is checked just above, so the cast cannot saturate.
        true => Some(micro as u64),
        false => None,
    }
}

#[cfg(test)]
#[path = "usage_tests.rs"]
mod tests;
