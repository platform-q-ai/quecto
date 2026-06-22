use super::{AnthropicProvider, DomainError, Message, SseAccumulator};
use crate::domain::message::LlmResponse;
use crate::domain::provider::{ChatRequest, StreamEvent};

#[cfg(any(test, feature = "test-support"))]
impl AnthropicProvider {
    pub fn build_request_body_public(
        request: &ChatRequest<'_>,
    ) -> (Option<String>, serde_json::Value) {
        Self::build_request_body(request, false)
    }

    pub fn build_request_body_with_oauth(
        request: &ChatRequest<'_>,
        is_oauth: bool,
    ) -> (Option<String>, serde_json::Value) {
        Self::build_request_body(request, is_oauth)
    }

    pub fn build_messages_public(messages: &[Message]) -> (Option<String>, Vec<serde_json::Value>) {
        Self::build_messages(messages, "claude-opus-4-5", false)
    }

    pub fn build_messages_for_model_public(
        messages: &[Message],
        model: &str,
    ) -> (Option<String>, Vec<serde_json::Value>) {
        Self::build_messages(messages, model, false)
    }

    pub fn parse_sse_response_public(raw: &str) -> Result<LlmResponse, DomainError> {
        Self::parse_sse_response(raw, None)
    }

    pub fn parse_sse_response_with_tools_public(
        raw: &str,
        tool_defs: &[crate::domain::tool::ToolDefinition],
    ) -> Result<LlmResponse, DomainError> {
        Self::parse_sse_response(raw, Some(tool_defs.to_vec()))
    }

    fn parse_sse_events(
        raw: &str,
        tool_defs: Option<Vec<crate::domain::tool::ToolDefinition>>,
    ) -> Vec<StreamEvent> {
        let mut events: Vec<StreamEvent> = Vec::new();
        let mut acc = match tool_defs {
            Some(defs) => SseAccumulator::with_tool_defs(defs),
            None => SseAccumulator::default(),
        };
        let mut current_event = String::new();
        for line in raw.lines() {
            let line = line.trim();
            if let Some(event_type) = line.strip_prefix("event: ") {
                current_event = event_type.to_string();
                continue;
            }
            if let Some(data) = line.strip_prefix("data: ") {
                let chunk: serde_json::Value = serde_json::from_str(data).unwrap_or_default();
                if Self::collect_sse_event(current_event.as_str(), &chunk, &mut acc, &mut events) {
                    break;
                }
            }
        }
        events.push(StreamEvent::Done(acc.into_response()));
        events
    }

    fn collect_sse_event(
        event_type: &str,
        chunk: &serde_json::Value,
        acc: &mut SseAccumulator,
        events: &mut Vec<StreamEvent>,
    ) -> bool {
        use super::anthropic_sse::stream_event_from_delta;
        match event_type {
            "message_start" => acc.handle_message_start(chunk),
            "content_block_start" => {
                acc.handle_block_start(chunk);
                if acc.in_tool_input {
                    events.push(StreamEvent::ToolCallStart {
                        id: acc.current_tool_id.clone(),
                        name: acc.current_tool_name.clone(),
                    });
                }
            }
            "content_block_delta" => {
                if let Some(ev) = stream_event_from_delta(&chunk["delta"]) {
                    events.push(ev);
                }
                acc.handle_block_delta(chunk);
            }
            "content_block_stop" => {
                if acc.in_tool_input {
                    events.push(StreamEvent::ToolCallEnd {
                        id: acc.current_tool_id.clone(),
                        name: acc.current_tool_name.clone(),
                        arguments: acc.current_tool_input.clone(),
                    });
                }
                acc.handle_block_stop();
            }
            "message_delta" => acc.handle_message_delta(chunk),
            "message_stop" => return true,
            _ => {}
        }
        false
    }

    pub fn parse_sse_events_public(raw: &str) -> Vec<StreamEvent> {
        Self::parse_sse_events(raw, None)
    }

    pub fn parse_sse_events_with_tools_public(
        raw: &str,
        tool_defs: &[crate::domain::tool::ToolDefinition],
    ) -> Vec<StreamEvent> {
        Self::parse_sse_events(raw, Some(tool_defs.to_vec()))
    }

    pub fn build_tool_result_message_public(m: &Message) -> serde_json::Value {
        serde_json::json!({
            "role": "user",
            "content": [Self::build_tool_result_block(m)],
        })
    }

    pub fn build_beta_header_public(model: &str, is_oauth: bool) -> String {
        Self::build_beta_header(model, is_oauth)
    }

    pub fn to_claude_code_name_public(name: &str) -> &str {
        super::to_claude_code_name(name)
    }
}
