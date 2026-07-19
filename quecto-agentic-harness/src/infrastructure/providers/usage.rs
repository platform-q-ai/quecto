//! Consolidated `UsageInfo` JSON parsers for the OpenAI/Codex provider family.
//!
//! Historically five near-identical inline parsers extracted token usage from
//! provider responses (two OpenAI chat shapes, two Codex Responses shapes, and
//! the SSE aggregator). They drifted apart over time. These two entry points
//! collapse that duplication into a single place per wire shape.

use crate::domain::message::{ModelPricing, UsageInfo, model_pricing};
use crate::infrastructure::model_registry::{ModelCost, ModelRegistry};
use serde_json::{Map, Value};

/// Read a `u32` token count from a usage object, defaulting to 0.
fn u32_field(obj: &Map<String, Value>, key: &str) -> u32 {
    obj.get(key)
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(0)
}

/// Read an optional `u32` token count from a usage object.
fn opt_u32_field(obj: &Map<String, Value>, key: &str) -> Option<u32> {
    obj.get(key)
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
}

/// Parse an OpenAI chat-completions `usage` object.
///
/// OpenAI's `prompt_tokens` already counts the full prompt (cached tokens are
/// a subset), so `context_tokens` is taken from `total_tokens` when present.
pub fn parse_openai_usage(obj: &Map<String, Value>) -> UsageInfo {
    parse_openai_usage_inner(obj, None)
}

pub fn parse_openai_usage_for_model(obj: &Map<String, Value>, model: &str) -> UsageInfo {
    parse_openai_usage_inner(obj, pricing_for_model(model))
}

fn parse_openai_usage_inner(obj: &Map<String, Value>, pricing: Option<ModelPricing>) -> UsageInfo {
    let mut usage = UsageInfo {
        prompt_tokens: u32_field(obj, "prompt_tokens"),
        completion_tokens: u32_field(obj, "completion_tokens"),
        cache_read_tokens: obj
            .get("prompt_tokens_details")
            .and_then(|d| d.get("cached_tokens"))
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok()),
        cache_write_tokens: None,
        context_tokens: opt_u32_field(obj, "total_tokens"),
        cost: None,
    };
    usage.cost = pricing.map(|p| p.cost_for(&usage));
    usage
}

/// Parse a Codex/OpenAI Responses `usage` object.
///
/// Codex `input_tokens` already counts the full prompt (cached tokens are a
/// subset), so `context_tokens` is left `None` and the gauge falls back to
/// `prompt_tokens`. Cached tokens are reported under `input_tokens_details`.
pub fn parse_codex_usage(obj: &Map<String, Value>) -> UsageInfo {
    parse_codex_usage_inner(obj, None)
}

pub fn parse_codex_usage_for_model(obj: &Map<String, Value>, model: &str) -> UsageInfo {
    parse_codex_usage_inner(obj, pricing_for_model(model))
}

fn parse_codex_usage_inner(obj: &Map<String, Value>, pricing: Option<ModelPricing>) -> UsageInfo {
    let cache_read_tokens = obj
        .get("input_tokens_details")
        .and_then(|d| d.get("cached_tokens"))
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok());
    let mut usage = UsageInfo {
        prompt_tokens: u32_field(obj, "input_tokens"),
        completion_tokens: u32_field(obj, "output_tokens"),
        cache_read_tokens,
        cache_write_tokens: None,
        context_tokens: None,
        cost: None,
    };
    usage.cost = pricing.map(|p| p.cost_for(&usage));
    usage
}

fn pricing_for_model(model: &str) -> Option<ModelPricing> {
    model_pricing(model).or_else(|| {
        let (provider, id) = model.split_once('/')?;
        let cost = ModelRegistry::builtin().find(provider, id)?.cost.clone();
        model_cost_to_pricing(&cost)
    })
}

fn model_cost_to_pricing(cost: &ModelCost) -> Option<ModelPricing> {
    if cost.input == 0.0 && cost.output == 0.0 && cost.cache_read == 0.0 && cost.cache_write == 0.0
    {
        return None;
    }
    Some(ModelPricing {
        input_micro_usd_per_million: usd_per_million_to_micro(cost.input),
        output_micro_usd_per_million: usd_per_million_to_micro(cost.output),
        cache_read_micro_usd_per_million: usd_per_million_to_micro(cost.cache_read),
        cache_write_micro_usd_per_million: usd_per_million_to_micro(cost.cache_write),
    })
}

fn usd_per_million_to_micro(usd: f64) -> u64 {
    (usd * 1_000_000.0).round() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_reads_prompt_completion_and_total() {
        let v = serde_json::json!({
            "prompt_tokens": 12, "completion_tokens": 7, "total_tokens": 19
        });
        let u = parse_openai_usage(v.as_object().unwrap());
        assert_eq!(u.prompt_tokens, 12);
        assert_eq!(u.completion_tokens, 7);
        assert_eq!(u.context_tokens, Some(19));
        assert_eq!(u.cache_read_tokens, None);
    }

    #[test]
    fn openai_missing_total_leaves_context_none() {
        let v = serde_json::json!({ "prompt_tokens": 3, "completion_tokens": 4 });
        let u = parse_openai_usage(v.as_object().unwrap());
        assert_eq!(u.context_tokens, None);
    }

    #[test]
    fn codex_reads_input_output_and_cached() {
        let v = serde_json::json!({
            "input_tokens": 100, "output_tokens": 40,
            "input_tokens_details": { "cached_tokens": 30 }
        });
        let u = parse_codex_usage(v.as_object().unwrap());
        assert_eq!(u.prompt_tokens, 100);
        assert_eq!(u.completion_tokens, 40);
        assert_eq!(u.cache_read_tokens, Some(30));
        assert_eq!(u.context_tokens, None);
    }

    #[test]
    fn openai_priced_model_populates_cost() {
        let v = serde_json::json!({
            "prompt_tokens": 1_000_000,
            "completion_tokens": 1_000_000,
            "total_tokens": 2_000_000,
            "prompt_tokens_details": { "cached_tokens": 500_000 }
        });
        let u = parse_openai_usage_for_model(v.as_object().unwrap(), "openai-api/gpt-5.6-luna");
        let cost = u.cost.unwrap();
        assert_eq!(cost.input_cost_micro_usd, 1_000_000);
        assert_eq!(cost.output_cost_micro_usd, 6_000_000);
        assert_eq!(cost.cache_read_cost_micro_usd, 50_000);
        assert_eq!(cost.total_cost_micro_usd, 7_050_000);
    }

    #[test]
    fn unknown_model_leaves_cost_none() {
        let v = serde_json::json!({
            "prompt_tokens": 1_000_000,
            "completion_tokens": 1_000_000
        });
        let u = parse_openai_usage_for_model(v.as_object().unwrap(), "openai-api/not-priced");
        assert!(u.cost.is_none());
    }

    #[test]
    fn codex_priced_model_populates_cost() {
        let v = serde_json::json!({
            "input_tokens": 1_000_000,
            "output_tokens": 1_000_000,
            "input_tokens_details": { "cached_tokens": 500_000 }
        });
        let u = parse_codex_usage_for_model(v.as_object().unwrap(), "openai-api/gpt-5.6-luna");
        assert_eq!(u.cost.unwrap().total_cost_micro_usd, 7_050_000);
    }
}
