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
    /// The session's cost after this turn: the sum of every turn's
    /// charge, micro-USD.
    pub total_cost_micro_usd: u64,
    /// The process's cumulative total went down (to zero, or lower) without
    /// a new process: nothing was charged for it.
    pub cost_drop: Option<CostDrop>,
}

/// A cumulative total lower than the one before, inside one process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CostDrop {
    pub previous_micro_usd: u64,
    pub reported_micro_usd: u64,
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
    /// without a usable total leaves the total as it was, and a lower total
    /// is a new process charged its whole total.
    /// A new process started: its cumulative totals start from zero.
    pub fn process_started(&mut self) {}

    pub fn record(&mut self, result: &ResultEvent) -> TurnUsage {
        let previous = self.total_cost_micro_usd;
        let reported = result.total_cost_usd.and_then(micro_usd);
        // The total is external data: a drop means a new process (its
        // total restarted from zero), charged its whole total.
        let (total, cost, reset_from) = match reported {
            Some(total) if total >= previous => (total, total - previous, None),
            Some(total) => (total, total, Some(previous)),
            None => (previous, 0, None),
        };
        self.total_cost_micro_usd = total;
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
            total_cost_micro_usd: total,
            cost_drop: reset_from.map(|previous| CostDrop {
                previous_micro_usd: previous,
                reported_micro_usd: total,
            }),
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
    // 2^64 itself does not fit (`u64::MAX as f64` rounds up to it), so the
    // range is half-open (NaN and infinities are outside it); inside it the
    // cast is exact for these whole numbers.
    match (0.0..TWO_POW_64).contains(&micro) {
        true => Some(micro as u64),
        false => None,
    }
}

/// 2^64, the first integer a `u64` cannot hold.
const TWO_POW_64: f64 = 18_446_744_073_709_551_616.0;

#[cfg(test)]
#[path = "usage_tests.rs"]
mod tests;
