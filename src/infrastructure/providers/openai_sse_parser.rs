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
            usage = Some(UsageInfo {
                prompt_tokens: u
                    .get("prompt_tokens")
                    .and_then(|v| v.as_u64())
                    .and_then(|n| u32::try_from(n).ok())
                    .unwrap_or(0),
                completion_tokens: u
                    .get("completion_tokens")
                    .and_then(|v| v.as_u64())
                    .and_then(|n| u32::try_from(n).ok())
                    .unwrap_or(0),
                cache_read_tokens: None,
                cache_write_tokens: None,
                context_tokens: u
                    .get("total_tokens")
                    .and_then(|v| v.as_u64())
                    .and_then(|n| u32::try_from(n).ok()),
                cost: None,
            });
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
mod tests {
    use super::*;

    #[test]
    fn test_parse_sse_text_response() {
        let sse = "\
data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\
data: {\"choices\":[{\"delta\":{\"content\":\" world\"}}]}\n\
data: [DONE]\n";
        let result = parse_sse_response(sse).unwrap();
        assert_eq!(result.content.as_deref(), Some("Hello world"));
        assert!(result.tool_calls.is_empty());
    }

    #[test]
    fn test_parse_sse_tool_call_response() {
        let sse = "\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"bash\",\"arguments\":\"\"}}]}}]}\n\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"cmd\\\"\"}}]}}]}\n\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\": \\\"ls\\\"}\"}}]}}]}\n\
data: [DONE]\n";
        let result = parse_sse_response(sse).unwrap();
        assert!(result.content.is_none());
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].id, "call_1");
        assert_eq!(result.tool_calls[0].name, "bash");
        assert!(result.tool_calls[0].arguments.contains("ls"));
    }

    #[test]
    fn test_parse_sse_empty() {
        let sse = "data: [DONE]\n";
        let result = parse_sse_response(sse).unwrap();
        assert!(result.content.is_none());
        assert!(result.tool_calls.is_empty());
    }

    #[test]
    fn parse_sse_response_extracts_usage_chunk() {
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n",
            "data: {\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5,\"total_tokens\":15}}\n\n",
            "data: [DONE]\n\n",
        );
        let response = parse_sse_response(sse).unwrap();
        assert_eq!(response.content.as_deref(), Some("Hello"));
        let usage = response.usage.expect("usage chunk should be captured");
        assert_eq!(usage.prompt_tokens, 10);
        assert_eq!(usage.completion_tokens, 5);
    }
}
