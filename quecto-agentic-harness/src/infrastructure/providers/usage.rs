//! Consolidated `UsageInfo` JSON parsers for the OpenAI/Codex provider family.
//!
//! Historically five near-identical inline parsers extracted token usage from
//! provider responses (two OpenAI chat shapes, two Codex Responses shapes, and
//! the SSE aggregator). They drifted apart over time. These two entry points
//! collapse that duplication into a single place per wire shape.

use crate::domain::message::UsageInfo;
use serde_json::{Map, Value};

/// Read a `u32` token count from a usage object, defaulting to 0.
fn u32_field(obj: &Map<String, Value>, key: &str) -> u32 {
    obj.get(key)
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(0)
}

fn cached_tokens(obj: &Map<String, Value>, details_key: &str) -> Option<u32> {
    obj.get(details_key)
        .and_then(|d| d.get("cached_tokens"))
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
}

fn normalize_cached_input(
    provider_input_tokens: u32,
    cache_read_tokens: Option<u32>,
) -> (u32, Option<u32>) {
    let cache_read_tokens = cache_read_tokens.map(|n| n.min(provider_input_tokens));
    (
        provider_input_tokens.saturating_sub(cache_read_tokens.unwrap_or(0)),
        cache_read_tokens,
    )
}

/// Parse an OpenAI chat-completions `usage` object.
///
/// OpenAI's `prompt_tokens` counts the full prompt and cached tokens are a
/// subset. Normalize `prompt_tokens` to full-price, non-cache input for shared
/// billing/cost accounting, and carry the provider full prompt as context.
pub fn parse_openai_usage(obj: &Map<String, Value>) -> UsageInfo {
    let provider_prompt_tokens = u32_field(obj, "prompt_tokens");
    let (prompt_tokens, cache_read_tokens) = normalize_cached_input(
        provider_prompt_tokens,
        cached_tokens(obj, "prompt_tokens_details"),
    );
    UsageInfo {
        prompt_tokens,
        completion_tokens: u32_field(obj, "completion_tokens"),
        cache_read_tokens,
        cache_write_tokens: None,
        context_tokens: Some(provider_prompt_tokens),
        cost: None,
    }
}

/// Parse a Codex/OpenAI Responses `usage` object.
///
/// Codex/OpenAI Responses `input_tokens` counts the full prompt and cached
/// tokens are a subset. Normalize `prompt_tokens` to full-price, non-cache
/// input for shared billing/cost accounting, and carry provider input as
/// context occupancy.
pub fn parse_codex_usage(obj: &Map<String, Value>) -> UsageInfo {
    let provider_input_tokens = u32_field(obj, "input_tokens");
    let (prompt_tokens, cache_read_tokens) = normalize_cached_input(
        provider_input_tokens,
        cached_tokens(obj, "input_tokens_details"),
    );
    UsageInfo {
        prompt_tokens,
        completion_tokens: u32_field(obj, "output_tokens"),
        cache_read_tokens,
        cache_write_tokens: None,
        context_tokens: Some(provider_input_tokens),
        cost: None,
    }
}

#[cfg(test)]
#[path = "usage_tests.rs"]
mod tests;
