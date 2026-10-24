use crate::domain::conversation::value_objects::message::UsageInfo;
use crate::domain::inference::value_objects::model_price::ModelPrice;

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

/// Cost normalized `usage` at the model's catalogue price (#1855): the one
/// price authority. A model with no price leaves the usage uncosted.
///
/// Provider adapters own wire parsing and never price; the agent turn
/// prices usage once it is normalized, from the active model's catalogue
/// entry.
pub fn price(usage: &mut UsageInfo, price: Option<ModelPrice>) {
    usage.cost = price.map(|price| price.cost_of(usage));
}
