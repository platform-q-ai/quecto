//! Parsing Anthropic non-stream message responses.
use super::*;
use crate::domain::message::{StopReason, ThinkingBlock, ToolCall};
use crate::domain::visible_thinking::append_visible_thinking;

impl AnthropicProvider {
    pub(super) fn parse_response(
        body: &serde_json::Value,
        is_oauth: bool,
        tools: &[crate::domain::tool::ToolDefinition],
    ) -> Result<LlmResponse, DomainError> {
        let content_blocks = body["content"]
            .as_array()
            .ok_or_else(|| DomainError::Provider("missing content in response".to_string()))?;

        let mut text_parts: Vec<String> = Vec::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        let mut thinking_blocks: Vec<ThinkingBlock> = Vec::new();
        let mut thinking_budget = String::new();

        for block in content_blocks {
            match block["type"].as_str() {
                Some("text") => {
                    if let Some(t) = block["text"].as_str() {
                        text_parts.push(t.to_string());
                    }
                }
                Some("thinking") => {
                    let thinking = block["thinking"].as_str().unwrap_or_default();
                    if thinking.is_empty() {
                        continue;
                    }
                    append_visible_thinking(
                        &mut thinking_budget,
                        thinking,
                        "Anthropic non-stream thinking",
                    )?;
                    thinking_blocks.push(ThinkingBlock::Normal {
                        thinking: thinking.to_string(),
                        signature: block["signature"].as_str().unwrap_or_default().to_string(),
                    });
                }
                Some("redacted_thinking") => {
                    thinking_blocks.push(ThinkingBlock::Redacted {
                        data: block["data"].as_str().unwrap_or_default().to_string(),
                    });
                }
                Some("tool_use") => {
                    let id = block["id"].as_str().unwrap_or_default().to_string();
                    let raw_name = block["name"].as_str().unwrap_or_default().to_string();
                    // Reverse-map canonical tool names for OAuth (#437-4)
                    let name = if is_oauth {
                        claude_code::from_claude_code_name(&raw_name, tools)
                    } else {
                        raw_name
                    };
                    let input = &block["input"];
                    let arguments = serde_json::to_string(input).unwrap_or_default();
                    tool_calls.push(ToolCall {
                        id,
                        name,
                        arguments,
                    });
                }
                _ => {}
            }
        }

        let content = if text_parts.is_empty() {
            None
        } else {
            Some(text_parts.join(""))
        };

        let usage = body["usage"].as_object().map(usage::parse_usage);

        let stop_reason = body["stop_reason"].as_str().map(StopReason::parse);

        Ok(LlmResponse {
            content,
            tool_calls,
            usage,
            stop_reason,
            thinking_blocks,
        })
    }
}
