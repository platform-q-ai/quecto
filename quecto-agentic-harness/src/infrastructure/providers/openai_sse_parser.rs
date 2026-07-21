//! SSE response parser for OpenAI chat completions.
//!
//! Extracted from `openai.rs` to keep the provider file under the 750-line
//! limit and to make the parser independently testable.

use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, ToolCall, UsageInfo};

/// Maximum number of tool calls allowed in a single streaming response.
const MAX_TOOL_CALLS: usize = 128;

/// Parse an SSE text stream into an assembled `LlmResponse`.
///
/// Captures content deltas, tool-call deltas, and the final `usage` chunk
/// (when `stream_options.include_usage` is enabled).
pub(crate) fn parse_sse_response(raw: &str) -> Result<LlmResponse, DomainError> {
    let mut content = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    let mut usage: Option<UsageInfo> = None;

    for line in raw.lines() {
        let line = line.trim();
        let Some(data) = line.strip_prefix("data: ") else {
            continue;
        };
        if data == "[DONE]" {
            break;
        }
        let chunk: serde_json::Value = serde_json::from_str(data).unwrap_or_default();

        if let Some(choices) = chunk.get("choices").and_then(|v| v.as_array()) {
            for choice in choices {
                apply_delta(
                    choice.get("delta").unwrap_or(&serde_json::Value::Null),
                    &mut content,
                    &mut tool_calls,
                );
            }
        }

        if let Some(u) = chunk.get("usage").and_then(|v| v.as_object()) {
            usage = Some(crate::infrastructure::providers::usage::parse_openai_usage(
                u,
            ));
        }
    }

    let content = if content.is_empty() {
        None
    } else {
        Some(content)
    };

    Ok(LlmResponse {
        content,
        tool_calls,
        usage,
        stop_reason: None,
        thinking_blocks: vec![],
    })
}

/// Apply a single SSE delta chunk to the accumulated content and tool calls.
pub(crate) fn apply_delta(
    delta: &serde_json::Value,
    content: &mut String,
    tool_calls: &mut Vec<ToolCall>,
) {
    if let Some(text) = delta["content"].as_str() {
        content.push_str(text);
    }
    if let Some(tcs) = delta["tool_calls"].as_array() {
        for tc in tcs {
            let idx = tc["index"].as_u64().unwrap_or(0) as usize;
            if idx >= MAX_TOOL_CALLS {
                continue;
            }
            while tool_calls.len() <= idx {
                tool_calls.push(ToolCall {
                    id: String::new(),
                    name: String::new(),
                    arguments: String::new(),
                });
            }
            if let Some(id) = tc["id"].as_str() {
                tool_calls[idx].id = id.to_string();
            }
            if let Some(name) = tc["function"]["name"].as_str() {
                tool_calls[idx].name = name.to_string();
            }
            if let Some(args) = tc["function"]["arguments"].as_str() {
                tool_calls[idx].arguments.push_str(args);
            }
        }
    }
}

#[cfg(test)]
#[path = "openai_sse_parser_tests.rs"]
mod tests;
