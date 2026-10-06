use crate::domain::message::{LlmResponse, model_pricing};

/// Shared normalized cache-hit ratio for cumulative usage stats.
///
/// `input_tokens` is full-price, non-cache billable input. Cache writes count
/// toward prompt-cache context denominator but never as cache hits.
pub fn cache_hit_ratio(
    input_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
) -> Option<f64> {
    let denominator = input_tokens
        .saturating_add(cache_read_tokens)
        .saturating_add(cache_write_tokens);
    (denominator != 0).then_some(cache_read_tokens as f64 / denominator as f64)
}

/// Attach shared model-pricing cost to normalized usage, when pricing exists.
///
/// Provider adapters own wire parsing; cost math stays in the domain pricing
/// model and is applied after provider usage has been normalized.
pub fn attach_cost(response: &mut LlmResponse, model: &str) {
    if let (Some(usage), Some(pricing)) = (&mut response.usage, model_pricing(model)) {
        usage.cost = Some(pricing.cost_for(usage));
    }
}
