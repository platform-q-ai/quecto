//! A model's price, as usage is costed (#1855, #2441, #2328): the
//! catalogue's model cost, the one price authority, in integer micro-USD
//! per million tokens so costing never drifts with floating point.
use crate::domain::catalogue::value_objects::model_facts::ModelCost;
use crate::domain::conversation::value_objects::message::{CostInfo, UsageInfo};

/// Micro-USD per million tokens, by kind of token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelPrice {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl ModelPrice {
    /// The price of a model whose catalogue cost was stated (a published
    /// row or a `models.json` declaration): `None` for a cost nobody stated.
    /// A stated all-zero cost is a free model's price (#2635 review). Only
    /// finite, non-negative rates are a price; anything else is no price.
    pub fn of(cost: &ModelCost) -> Option<Self> {
        let rates = [cost.input, cost.output, cost.cache_read, cost.cache_write];
        let valid = rates.iter().all(|rate| rate.is_finite() && *rate >= 0.0);
        let declared = cost.declared;
        let micro = |usd: f64| (usd * 1_000_000.0).round() as u64;
        (valid && declared).then(|| Self {
            input: micro(cost.input),
            output: micro(cost.output),
            cache_read: micro(cost.cache_read),
            cache_write: micro(cost.cache_write),
        })
    }

    /// What `usage` cost at this price, in exact integer micro-USD.
    /// `prompt_tokens` is the full-price input: cache reads and writes are
    /// costed at their own rates, never twice. An absurd rate saturates
    /// rather than wrapping (#1855 review L2).
    pub fn cost_of(&self, usage: &UsageInfo) -> CostInfo {
        let calc = |tokens: u32, rate: u64| u64::from(tokens).saturating_mul(rate) / 1_000_000;
        let input = calc(usage.prompt_tokens, self.input);
        let output = calc(usage.completion_tokens, self.output);
        let cache_read = calc(usage.cache_read_tokens.unwrap_or(0), self.cache_read);
        let cache_write = calc(usage.cache_write_tokens.unwrap_or(0), self.cache_write);
        CostInfo {
            input_cost_micro_usd: input,
            output_cost_micro_usd: output,
            cache_read_cost_micro_usd: cache_read,
            cache_write_cost_micro_usd: cache_write,
            total_cost_micro_usd: input
                .saturating_add(output)
                .saturating_add(cache_read)
                .saturating_add(cache_write),
        }
    }
}

#[cfg(test)]
#[path = "model_price_tests.rs"]
mod tests;
