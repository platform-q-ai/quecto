//! What an external agent's turns used (#2285): `result.usage` is per
//! turn while `total_cost_usd` is cumulative for the process, so a turn's
//! cost is the growth of its process's total, kept in whole micro-USD and
//! rounded once per result; the session's cost is the sum of those.

use super::stream::{ResultEvent, TokenCounts};

/// What one turn used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TurnUsage {
    /// This turn's tokens (`result.usage`).
    pub tokens: TokenCounts,
    /// This turn's charge: its process total's growth, micro-USD.
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

/// Turns each process's cumulative totals into per-turn charges.
///
/// Within one process the total only grows, so a turn is charged its
/// delta. A zero or lower total inside a process (the CLI reports 0 after
/// a session crash, a delivery error or a bridge interrupt) is charged
/// nothing and reported as a [`CostDrop`]; the next higher total is charged
/// from the highest seen. Only [`UsageLedger::process_started`], which the
/// session calls when it spawns a process, starts the totals from zero.
/// The session's cost is the sum of the charges.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageLedger {
    /// The highest cumulative total the current process reported.
    process_total_micro_usd: u64,
    /// The sum of every turn's charge.
    session_cost_micro_usd: u64,
    /// The current process's cumulative tokens.
    process_tokens: TokenCounts,
    /// The tokens of the processes before it.
    earlier_tokens: TokenCounts,
}

impl UsageLedger {
    /// A new process started: its cumulative totals start from zero.
    pub fn process_started(&mut self) {
        self.earlier_tokens = self.earlier_tokens.plus(self.process_tokens);
        self.process_tokens = TokenCounts::default();
        self.process_total_micro_usd = 0;
    }

    /// Record one turn's `result`: charge the process total's growth,
    /// rounded once to micro-USD. A result without a usable total is
    /// charged nothing.
    pub fn record(&mut self, result: &ResultEvent) -> TurnUsage {
        let previous = self.process_total_micro_usd;
        let reported = result.total_cost_usd.and_then(micro_usd);
        let (cost, cost_drop) = match reported {
            Some(total) if total >= previous => (total - previous, None),
            Some(total) => (
                0,
                Some(CostDrop {
                    previous_micro_usd: previous,
                    reported_micro_usd: total,
                }),
            ),
            None => (0, None),
        };
        self.process_total_micro_usd = previous.max(reported.unwrap_or(0));
        self.session_cost_micro_usd = self.session_cost_micro_usd.saturating_add(cost);
        let process_tokens = result
            .model_usage
            .iter()
            .fold(TokenCounts::default(), |sum, model| sum.plus(model.tokens));
        self.process_tokens = match result.model_usage.is_empty() {
            true => self.process_tokens.plus(result.usage),
            false => process_tokens,
        };
        TurnUsage {
            tokens: result.usage,
            cost_micro_usd: cost,
            total_cost_micro_usd: self.session_cost_micro_usd,
            cost_drop,
        }
    }

    /// The session's cost: the sum of every turn's charge, micro-USD.
    pub fn total_cost_micro_usd(&self) -> u64 {
        self.session_cost_micro_usd
    }

    /// The session's tokens: every process's cumulative `modelUsage`.
    pub fn cumulative_tokens(&self) -> TokenCounts {
        self.earlier_tokens.plus(self.process_tokens)
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
