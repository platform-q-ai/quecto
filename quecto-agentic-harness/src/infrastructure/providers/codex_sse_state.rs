use std::collections::HashMap;

use crate::domain::message::{LlmResponse, StopReason, ToolCall, UsageInfo};

#[derive(Default)]
pub(super) struct SseAccumulator {
    pub(super) content: String,
    pub(super) tool_calls: Vec<ToolCall>,
    pub(super) output_index_to_tool: HashMap<usize, usize>,
    pub(super) usage: Option<UsageInfo>,
    pub(super) stop_reason: Option<StopReason>,
}

impl SseAccumulator {
    pub(super) fn has_observable_output(&self) -> bool {
        !self.content.is_empty() || !self.tool_calls.is_empty()
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
        LlmResponse {
            content: if self.content.is_empty() {
                None
            } else {
                Some(self.content)
            },
            tool_calls: self.tool_calls,
            usage: self.usage,
            stop_reason: self.stop_reason,
            thinking_blocks: vec![],
        }
    }
}
