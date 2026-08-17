use std::collections::HashMap;

use crate::domain::message::{LlmResponse, StopReason, ThinkingBlock, ToolCall, UsageInfo};
use crate::infrastructure::providers::openai::openai_sse_parser::{
    MAX_OPENAI_SSE_REASONING_BYTES, append_with_limit,
};
use serde_json::Value;

#[derive(Default)]
pub(super) struct SseAccumulator {
    pub(super) content: String,
    pub(super) tool_calls: Vec<ToolCall>,
    pub(super) output_index_to_tool: HashMap<usize, usize>,
    pub(super) usage: Option<UsageInfo>,
    pub(super) stop_reason: Option<StopReason>,
    pub(super) reasoning: String,
}

fn collect_reasoning_summary(item: &Value) -> String {
    let mut collected = String::new();
    if let Some(summary) = item.get("summary") {
        match summary {
            Value::String(text) => collected.push_str(text),
            Value::Array(parts) => {
                for part in parts {
                    if let Some(text) = part
                        .as_str()
                        .or_else(|| part.get("text").and_then(|v| v.as_str()))
                    {
                        collected.push_str(text);
                    }
                }
            }
            _ => {}
        }
    }
    collected
}

fn normalized_reasoning_suffix(text: &str) -> &str {
    text.trim_end_matches(char::is_whitespace)
}

fn reasoning_ends_with_summary(reasoning: &str, summary: &str) -> bool {
    let normalized_summary = normalized_reasoning_suffix(summary);
    !normalized_summary.is_empty()
        && normalized_reasoning_suffix(reasoning).ends_with(normalized_summary)
}

#[cfg(test)]
pub(super) fn append_reasoning_summary(item: &Value, reasoning: &mut String) {
    let _ = append_reasoning_with_limit(reasoning, &collect_reasoning_summary(item));
}

pub(super) fn append_reasoning_with_limit(
    reasoning: &mut String,
    text: &str,
) -> Result<(), crate::domain::error::DomainError> {
    append_with_limit(reasoning, text, MAX_OPENAI_SSE_REASONING_BYTES, "reasoning")
}

impl SseAccumulator {
    pub(super) fn has_observable_output(&self) -> bool {
        !self.content.is_empty() || !self.tool_calls.is_empty() || !self.reasoning.is_empty()
    }

    pub(super) fn parse_response_status(status: &str) -> StopReason {
        match status {
            "completed" => StopReason::EndTurn,
            "incomplete" => StopReason::MaxTokens,
            "failed" => StopReason::Error,
            other => StopReason::parse(other),
        }
    }

    pub(super) fn into_response(self) -> LlmResponse {
        let thinking_blocks = if self.reasoning.is_empty() {
            Vec::new()
        } else {
            vec![ThinkingBlock::Normal {
                thinking: self.reasoning,
                signature: String::new(),
            }]
        };
        LlmResponse {
            content: if self.content.is_empty() {
                None
            } else {
                Some(self.content)
            },
            tool_calls: self.tool_calls,
            usage: self.usage,
            stop_reason: self.stop_reason,
            thinking_blocks,
        }
    }
}

/// Accumulator for assembling Responses API SSE events into a response.
///
/// The Responses API emits `output_index` values that reflect the position
/// of each item in the full output array, which may include reasoning items
/// that are not tracked in our dense `tool_calls` vector. We maintain a
/// `HashMap<usize, usize>` mapping `output_index → tool_calls index` so
/// that `response.function_call_arguments.delta` events are routed to the
/// correct tool call regardless of intervening non-tool output items.
impl SseAccumulator {
    pub(super) fn handle_event(&mut self, event: &serde_json::Value) {
        match event["type"].as_str() {
            Some("response.output_text.delta") => {
                if let Some(delta) = event["delta"].as_str() {
                    self.content.push_str(delta);
                }
            }
            // Documented Responses refusal events (#1230): surface refusal
            // text as content with StopReason::Refusal, never an empty turn.
            Some("response.reasoning_summary_text.delta")
            | Some("response.reasoning.summary_text.delta") => {
                if let Some(delta) = event["delta"].as_str() {
                    let _ = append_reasoning_with_limit(&mut self.reasoning, delta);
                }
            }
            Some("response.output_item.done") => {
                if let Some(item) = event
                    .get("item")
                    .filter(|i| i["type"].as_str() == Some("reasoning"))
                {
                    let summary = collect_reasoning_summary(item);
                    if !summary.is_empty()
                        && !reasoning_ends_with_summary(&self.reasoning, &summary)
                    {
                        let _ = append_reasoning_with_limit(&mut self.reasoning, &summary);
                    }
                }
            }
            Some("response.refusal.delta") => {
                if let Some(delta) = event["delta"].as_str() {
                    self.content.push_str(delta);
                }
                self.stop_reason = Some(StopReason::Refusal);
            }
            Some("response.refusal.done") => {
                match event["refusal"].as_str() {
                    Some(refusal) if self.content.is_empty() => self.content.push_str(refusal),
                    _ => {}
                }
                self.stop_reason = Some(StopReason::Refusal);
            }
            Some("response.output_item.added") => self.handle_item_added(event),
            Some("response.function_call_arguments.delta") => {
                if let Some(delta) = event["delta"].as_str() {
                    let output_idx = event["output_index"].as_u64().unwrap_or(0) as usize;
                    if let Some(&tc_idx) = self.output_index_to_tool.get(&output_idx) {
                        if let Some(tc) = self.tool_calls.get_mut(tc_idx) {
                            tc.arguments.push_str(delta);
                        }
                    }
                }
            }
            Some("response.completed") => {
                if let Some(resp) = event.get("response") {
                    self.usage = resp["usage"]
                        .as_object()
                        .map(crate::infrastructure::providers::usage::parse_codex_usage);
                    if let Some(status) = resp["status"].as_str() {
                        self.stop_reason = Some(Self::parse_response_status(status));
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_item_added(&mut self, event: &serde_json::Value) {
        if let Some(item) = event.get("item") {
            if item["type"].as_str() == Some("function_call") {
                let output_idx = event["output_index"].as_u64().unwrap_or(0) as usize;
                let tc_idx = self.tool_calls.len();
                self.output_index_to_tool.insert(output_idx, tc_idx);
                self.tool_calls.push(ToolCall {
                    id: item["call_id"].as_str().unwrap_or_default().to_string(),
                    name: item["name"].as_str().unwrap_or_default().to_string(),
                    arguments: String::new(),
                });
            }
        }
    }
}
