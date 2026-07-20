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
    UsageInfo {
        prompt_tokens: u32_field(obj, "prompt_tokens"),
        completion_tokens: u32_field(obj, "completion_tokens"),
        cache_read_tokens: None,
        cache_write_tokens: None,
        context_tokens: opt_u32_field(obj, "total_tokens"),
        cost: None,
    }
}

/// Parse a Codex/OpenAI Responses `usage` object.
///
/// Codex `input_tokens` already counts the full prompt (cached tokens are a
/// subset), so `context_tokens` is left `None` and the gauge falls back to
/// `prompt_tokens`. Cached tokens are reported under `input_tokens_details`.
pub fn parse_codex_usage(obj: &Map<String, Value>) -> UsageInfo {
    let cache_read_tokens = obj
        .get("input_tokens_details")
        .and_then(|d| d.get("cached_tokens"))
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok());
    UsageInfo {
        prompt_tokens: u32_field(obj, "input_tokens"),
        completion_tokens: u32_field(obj, "output_tokens"),
        cache_read_tokens,
        cache_write_tokens: None,
        context_tokens: None,
        cost: None,
    }
}

#[cfg(test)]
#[path = "usage_tests.rs"]
mod tests;
