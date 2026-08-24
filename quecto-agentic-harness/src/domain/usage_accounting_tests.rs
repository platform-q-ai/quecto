use crate::domain::message::{LlmResponse, UsageInfo};
use crate::domain::usage_accounting::{attach_cost, cache_hit_ratio};

fn response_with_usage(usage: UsageInfo) -> LlmResponse {
    LlmResponse {
        content: Some("ok".to_string()),
        tool_calls: Vec::new(),
        usage: Some(usage),
        stop_reason: None,
        thinking_blocks: Vec::new(),
    }
}

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

#[test]
fn attach_cost_prices_normalized_openai_usage() {
    let mut response = response_with_usage(normalized_usage());

    attach_cost(&mut response, "gpt-5.6-luna");

    let cost = response.usage.unwrap().cost.expect("gpt-5.6 pricing");
    assert_eq!(cost.input_cost_micro_usd, 70);
    assert_eq!(cost.output_cost_micro_usd, 120);
    assert_eq!(cost.cache_read_cost_micro_usd, 3);
    assert_eq!(cost.cache_write_cost_micro_usd, 6);
    assert_eq!(cost.total_cost_micro_usd, 199);
}

#[test]
fn attach_cost_leaves_unknown_model_cost_absent_but_preserves_usage() {
    let mut response = response_with_usage(normalized_usage());

    attach_cost(&mut response, "unknown-openai-model");

    let usage = response.usage.unwrap();
    assert_eq!(usage.prompt_tokens, 70);
    assert_eq!(usage.completion_tokens, 20);
    assert_eq!(usage.cache_read_tokens, Some(30));
    assert_eq!(usage.context_tokens, Some(105));
    assert!(usage.cost.is_none());
}

#[test]
fn attach_cost_noops_without_usage() {
    let mut response = LlmResponse {
        content: None,
        tool_calls: Vec::new(),
        usage: None,
        stop_reason: None,
        thinking_blocks: Vec::new(),
    };

    attach_cost(&mut response, "gpt-5.6-luna");

    assert!(response.usage.is_none());
}
