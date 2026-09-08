//! Responses SSE event handler.
use super::codex_sse_state;
use super::{CodexProvider, SseAccumulator};
use crate::domain::message::LlmResponse;
use crate::domain::provider::StreamEvent;
use crate::infrastructure::providers::sse_common::{SseHandler, SseLineOutcome};

/// SSE line handler for the Codex Responses API.
pub(super) struct CodexSseHandler {
    acc: SseAccumulator,
    saw_terminal: bool,
    model: Option<String>,
}

impl CodexSseHandler {
    pub(super) fn new() -> Self {
        Self {
            acc: SseAccumulator::default(),
            saw_terminal: false,
            model: None,
        }
    }

    pub(super) fn with_model(model: impl Into<String>) -> Self {
        let mut handler = Self::new();
        handler.model = Some(model.into());
        handler
    }

    fn take_response(&mut self) -> LlmResponse {
        let mut response = std::mem::take(&mut self.acc).into_response();
        if let Some(model) = &self.model {
            crate::domain::usage_accounting::attach_cost(&mut response, model);
        }
        response
    }
}

impl SseHandler for CodexSseHandler {
    async fn process_line(
        &mut self,
        line: &str,
        tx: &tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> SseLineOutcome {
        let Some(data) = line.strip_prefix("data: ") else {
            return SseLineOutcome::Continue;
        };
        if data == "[DONE]" {
            self.saw_terminal = true;
            let _ = tx.send(StreamEvent::Done(self.take_response())).await;
            return SseLineOutcome::Done;
        }
        if let Ok(event) = serde_json::from_str::<serde_json::Value>(data) {
            if let Some(error) = CodexProvider::format_stream_failure(&event) {
                self.saw_terminal = true;
                let _ = tx.send(StreamEvent::Error(error)).await;
                return SseLineOutcome::Done;
            }
            match event["type"].as_str() {
                Some("response.output_text.delta") => {
                    if let Some(delta) = event["delta"].as_str() {
                        let _ = tx.send(StreamEvent::TextDelta(delta.to_string())).await;
                    }
                }
                Some("response.reasoning_summary_text.delta")
                | Some("response.reasoning.summary_text.delta") => {
                    if let Some(delta) = event["delta"].as_str() {
                        let position = codex_sse_state::reasoning_summary_position(&event);
                        let emitted = match codex_sse_state::append_reasoning_delta(
                            &mut self.acc.reasoning,
                            delta,
                            position,
                            &mut self.acc.reasoning_summary_position,
                        ) {
                            Ok(emitted) => emitted,
                            Err(err) => {
                                self.saw_terminal = true;
                                let _ = tx.send(StreamEvent::Error(err.to_string())).await;
                                return SseLineOutcome::Done;
                            }
                        };
                        let _ = tx.send(StreamEvent::ThinkingDelta(emitted)).await;
                    }
                }
                _ => {}
            }
            if !matches!(
                event["type"].as_str(),
                Some("response.reasoning_summary_text.delta")
                    | Some("response.reasoning.summary_text.delta")
            ) {
                if let Err(err) = self.acc.handle_event(&event) {
                    self.saw_terminal = true;
                    let _ = tx.send(StreamEvent::Error(err.to_string())).await;
                    return SseLineOutcome::Done;
                }
            }
            if event["type"].as_str() == Some("response.completed") {
                self.saw_terminal = true;
                let _ = tx.send(StreamEvent::Done(self.take_response())).await;
                return SseLineOutcome::Done;
            }
        }
        SseLineOutcome::Continue
    }

    async fn on_eof(&mut self, tx: &tokio::sync::mpsc::Sender<StreamEvent>) {
        if !self.saw_terminal && !self.acc.has_observable_output() {
            let _ = tx
                .send(StreamEvent::Error(
                    "Responses stream ended without completion".to_string(),
                ))
                .await;
            return;
        }
        let _ = tx.send(StreamEvent::Done(self.take_response())).await;
    }
}
