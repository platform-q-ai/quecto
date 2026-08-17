//! SSE response parser for OpenAI chat completions.
//!
//! Extracted from `openai.rs` to keep the provider file under the 750-line
//! limit and to make the parser independently testable.

use crate::domain::error::DomainError;
use crate::domain::message::ToolCall;
#[cfg(test)]
use crate::domain::message::{LlmResponse, ThinkingBlock, UsageInfo};

/// Maximum number of tool calls allowed in a single streaming response.
const MAX_TOOL_CALLS: usize = 128;

/// Maximum accumulated assistant content bytes for one OpenAI SSE response.
///
/// 8 MiB is far above legitimate chat completions while keeping a pathological
/// stream orders of magnitude below the RSS growth seen in #1201. The limit is
/// byte-based because `String` growth and provider payloads are byte-sized.
pub(crate) const MAX_OPENAI_SSE_CONTENT_BYTES: usize = 8 * 1024 * 1024;

/// Maximum accumulated `function.arguments` bytes for one tool call.
///
/// 2 MiB comfortably covers normal JSON tool payloads but prevents a single
/// adversarial tool-call argument accumulator from growing without bound.
pub(crate) const MAX_OPENAI_SSE_TOOL_ARGUMENT_BYTES: usize = 2 * 1024 * 1024;

/// Maximum accumulated display-safe reasoning bytes for one OpenAI-compatible
/// SSE response. Lower than the content cap because thinking is rendered live by
/// default and should remain a bounded summary, not an unbounded transcript.
pub(crate) const MAX_OPENAI_SSE_REASONING_BYTES: usize = 256 * 1024;

/// Parse an SSE text stream into an assembled `LlmResponse`.
///
/// Captures content deltas, tool-call deltas, and the final `usage` chunk
/// (when `stream_options.include_usage` is enabled).
#[cfg(test)]
pub(crate) fn parse_sse_response(raw: &str) -> Result<LlmResponse, DomainError> {
    let mut content = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    let mut thinking_blocks: Vec<ThinkingBlock> = Vec::new();
    let mut thinking_bytes = 0usize;
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
                let delta = choice.get("delta").unwrap_or(&serde_json::Value::Null);
                for thinking in
                    crate::infrastructure::providers::openai::supported_reasoning_fields(delta)
                {
                    thinking_bytes = thinking_bytes.saturating_add(thinking.len());
                    if thinking_bytes > MAX_OPENAI_SSE_REASONING_BYTES {
                        return Err(DomainError::Provider(format!(
                            "OpenAI SSE reasoning exceeds {MAX_OPENAI_SSE_REASONING_BYTES} byte limit"
                        )));
                    }
                    thinking_blocks.push(ThinkingBlock::Normal {
                        thinking: thinking.to_string(),
                        signature: String::new(),
                    });
                }
                apply_delta(delta, &mut content, &mut tool_calls)?;
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
        thinking_blocks,
    })
}

/// Apply a single SSE delta chunk to the accumulated content and tool calls.
pub(crate) fn apply_delta(
    delta: &serde_json::Value,
    content: &mut String,
    tool_calls: &mut Vec<ToolCall>,
) -> Result<(), DomainError> {
    if let Some(text) = delta["content"].as_str() {
        append_with_limit(
            content,
            text,
            MAX_OPENAI_SSE_CONTENT_BYTES,
            "assistant content",
        )?;
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
                append_with_limit(
                    &mut tool_calls[idx].arguments,
                    args,
                    MAX_OPENAI_SSE_TOOL_ARGUMENT_BYTES,
                    "tool-call arguments",
                )?;
            }
        }
    }
    Ok(())
}

pub(crate) fn append_with_limit(
    target: &mut String,
    fragment: &str,
    limit: usize,
    label: &str,
) -> Result<(), DomainError> {
    let new_len = target.len().checked_add(fragment.len()).ok_or_else(|| {
        DomainError::Provider(format!("OpenAI SSE {label} exceeds {limit} byte limit"))
    })?;
    if new_len > limit {
        return Err(DomainError::Provider(format!(
            "OpenAI SSE {label} exceeds {limit} byte limit"
        )));
    }
    target.push_str(fragment);
    Ok(())
}

#[cfg(test)]
#[path = "openai_sse_parser_tests.rs"]
mod tests;
