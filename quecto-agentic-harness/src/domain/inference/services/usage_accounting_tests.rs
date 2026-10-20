use crate::domain::conversation::value_objects::message::UsageInfo;
use crate::domain::inference::services::usage_accounting::{cache_hit_ratio, price};
use crate::domain::inference::value_objects::model_price::ModelPrice;

fn normalized_usage() -> UsageInfo {
    UsageInfo {
        prompt_tokens: 70,
        completion_tokens: 20,
        cache_read_tokens: Some(30),
        cache_write_tokens: Some(5),
        context_tokens: Some(105),
        cost: None,
    }
}

#[test]
fn cache_hit_ratio_uses_shared_normalized_denominator() {
    assert_eq!(cache_hit_ratio(0, 0, 0), None);
    assert_eq!(cache_hit_ratio(70, 0, 0), Some(0.0));
    assert_eq!(cache_hit_ratio(0, 30, 0), Some(1.0));
    assert_eq!(cache_hit_ratio(0, 30, 20), Some(30.0 / 50.0));
    assert_eq!(cache_hit_ratio(70, 30, 5), Some(30.0 / 105.0));
}

/// GPT-5.6 Luna's catalogue rates ($1 in, $6 out, $0.10 read, $1.25 write).
fn luna() -> ModelPrice {
    ModelPrice {
        input: 1_000_000,
        output: 6_000_000,
        cache_read: 100_000,
        cache_write: 1_250_000,
    }
}

#[test]
fn usage_is_costed_at_the_catalogue_price() {
    let mut usage = normalized_usage();
    price(&mut usage, Some(luna()));
    let cost = usage.cost.expect("priced");
    assert_eq!(cost.input_cost_micro_usd, 70);
    assert_eq!(cost.output_cost_micro_usd, 120);
    assert_eq!(cost.cache_read_cost_micro_usd, 3);
    assert_eq!(cost.cache_write_cost_micro_usd, 6);
    assert_eq!(cost.total_cost_micro_usd, 199);
}

#[test]
fn a_model_without_a_price_leaves_usage_uncosted_and_whole() {
    let mut usage = normalized_usage();
    usage.cost = Some(luna().cost_of(&usage));
    price(&mut usage, None);
    assert_eq!(usage.prompt_tokens, 70);
    assert_eq!(usage.completion_tokens, 20);
    assert_eq!(usage.cache_read_tokens, Some(30));
    assert_eq!(usage.context_tokens, Some(105));
    assert!(usage.cost.is_none(), "no price, no cost: never a stale one");
}
