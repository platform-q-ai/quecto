//! Responses SSE event handler.
use super::codex_sse_state;
use super::{CodexProvider, SseAccumulator};
use crate::domain::conversation::value_objects::message::LlmResponse;
use crate::domain::inference::value_objects::provider::StreamEvent;
use crate::infrastructure::providers::sse_common::{SseHandler, SseLineOutcome};
use crate::infrastructure::providers::sse_end;

/// The error a Responses body ends with when it ends before a terminal
/// event (`response.completed` or `[DONE]`), output or not (#2249 review):
/// a reply cut short, worded as the transport cut it is, so the retry
/// classifier reads it as a retryable network failure.
pub(crate) const RESPONSES_CUT_SHORT: &str =
    "Responses stream ended without completion: connection closed before response.completed";

/// SSE line handler for the Codex Responses API.
pub(super) struct CodexSseHandler {
    acc: SseAccumulator,
    saw_terminal: bool,
    /// Whether any `data:` event came: a body with none is an empty stream.
    saw_event: bool,
    model: Option<String>,
    /// Where reasoning items are replayed to (#2162).
    origin: String,
    /// Where usage reported before a cut is recorded (#2249 review).
    trace:
        Option<std::sync::Arc<crate::domain::inference::events::request_observation::RequestTrace>>,
}

impl CodexSseHandler {
    pub(super) fn new() -> Self {
        Self {
            acc: SseAccumulator::default(),
            saw_terminal: false,
            saw_event: false,
            model: None,
            origin: String::new(),
            trace: None,
        }
    }

    pub(super) fn with_model(model: impl Into<String>, origin: impl Into<String>) -> Self {
        let mut handler = Self::new();
        handler.model = Some(model.into());
        handler.origin = origin.into();
        handler
    }

    /// Record usage a cut-short reply reported on `trace` (#2249 review).
    pub(super) fn with_trace(
        mut self,
        trace: Option<
            std::sync::Arc<crate::domain::inference::events::request_observation::RequestTrace>,
        >,
    ) -> Self {
        self.trace = trace;
        self
    }

    fn take_response(&mut self) -> LlmResponse {
        let mut response = std::mem::take(&mut self.acc).into_response();
        if let Some(model) = &self.model {
            CodexProvider::finish_response(&mut response, model, &self.origin);
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
        let Some(data) = crate::infrastructure::providers::sse_common::event_data(line) else {
            return SseLineOutcome::Continue;
        };
        self.saw_event = true;
        if sse_end::is_done_marker(data) {
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
        // The pump ends the body here only when no line ended the stream,
        // so no terminal event was seen: whatever output came, the reply
        // was cut short, or, with no event at all, the stream was empty
        // (#2249 review).
        assert!(
            !self.saw_terminal,
            "a Responses stream ends at end of file only without a terminal event"
        );
        // Tokens a `response.*` event already reported were spent all the same.
        let usage = std::mem::take(&mut self.acc).into_response().usage;
        sse_end::record_unfinished_usage(
            self.trace.as_deref(),
            usage,
            self.model.as_deref().unwrap_or_default(),
        );
        let error = sse_end::ended_early(self.saw_event, RESPONSES_CUT_SHORT);
        let _ = tx.send(StreamEvent::Error(error)).await;
    }
}
