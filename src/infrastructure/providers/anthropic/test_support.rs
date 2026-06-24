#[cfg(any(test, feature = "test-support"))]
use super::{AnthropicProvider, DomainError, Message};
use crate::domain::message::LlmResponse;
use crate::domain::provider::{ChatRequest, StreamEvent};

#[cfg(any(test, feature = "test-support"))]
use super::anthropic_sse::AnthropicSseHandler;
#[cfg(any(test, feature = "test-support"))]
use crate::infrastructure::providers::sse_common::{SseHandler, SseLineOutcome};

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

    /// Drive the real Anthropic SSE line handler over a raw SSE payload.
    ///
    /// This is the test entry point for streaming event sequences. It exercises
    /// the production `AnthropicSseHandler::process_line` and `dispatch_sse_event`
    /// path rather than a hand-rolled copy of the dispatch logic, so production
    /// changes to event ordering, Done signalling, and tool-call remapping are
    /// reflected in the tests.
    pub async fn parse_sse_events_public(raw: &str) -> Vec<StreamEvent> {
        Self::parse_sse_events_with_tools_public(raw, &[]).await
    }

    pub async fn parse_sse_events_with_tools_public(
        raw: &str,
        tool_defs: &[crate::domain::tool::ToolDefinition],
    ) -> Vec<StreamEvent> {
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let mut handler = AnthropicSseHandler::new_for_test(Some(tool_defs.to_vec()));
        for line in raw.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if matches!(handler.process_line(line, &tx).await, SseLineOutcome::Done) {
                break;
            }
        }
        // Collect all events emitted by the real handler; if the handler never
        // sent a Done (e.g. no message_stop), flush the accumulated response.
        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        if !events.iter().any(|e| matches!(e, StreamEvent::Done(_))) {
            let _ = tx.send(StreamEvent::Done(handler.into_response())).await;
            if let Ok(ev) = rx.try_recv() {
                events.push(ev);
            }
        }
        events
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
