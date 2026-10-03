//! Incremental SSE byte-stream parser for OpenAI chat completions.
//!
//! Extracted from `openai.rs` to keep both files under the 750-line limit.
//! Uses the shared SSE pump from [`sse_common`].

use crate::domain::message::{LlmResponse, ThinkingBlock, ToolCall, UsageInfo};
use crate::domain::provider::StreamEvent;
use crate::infrastructure::providers::sse_common::{SseHandler, SseLineOutcome};

use super::OpenAiProvider;
use super::openai_sse_parser::{MAX_OPENAI_SSE_CONTENT_BYTES, append_with_limit};
use crate::domain::visible_thinking::append_visible_thinking;

/// The error an OpenAI chat-completions body ends with when it ends before a
/// terminal signal (`[DONE]` or a chunk naming a `finish_reason`), output or
/// not (#2236): a reply cut short, worded as the transport cut it is, so the
/// retry classifier reads it as a retryable network failure.
pub(crate) const OPENAI_CUT_SHORT: &str =
    "OpenAI SSE stream ended without completion: connection closed before [DONE]";

/// SSE line handler for OpenAI chat completions.
pub(crate) struct OpenAiSseHandler {
    content: String,
    tool_calls: Vec<ToolCall>,
    usage: Option<UsageInfo>,
    reasoning: String,
    /// Reused sink for `apply_delta`'s content extraction (which we ignore
    /// here, since content is accumulated into `content` directly).
    delta_scratch: String,
    model: Option<String>,
    /// The latest `finish_reason` seen on any chunk (#2116).
    stop_reason: Option<crate::domain::message::StopReason>,
    /// A choice named a non-empty `finish_reason` (#2236): the reply is
    /// whole even if the body then ends without `[DONE]`, as some
    /// OpenAI-compatible servers end it.
    finished: bool,
    /// `[DONE]` ended the stream; the pump never reads past it.
    saw_done: bool,
    /// Whether any `data:` event came: a body with none is an empty stream.
    saw_event: bool,
    /// Where usage reported before a cut is recorded (#2249 review).
    trace: Option<std::sync::Arc<crate::domain::request_observation::RequestTrace>>,
}

impl OpenAiSseHandler {
    fn new() -> Self {
        Self {
            content: String::new(),
            tool_calls: Vec::new(),
            usage: None,
            reasoning: String::new(),
            delta_scratch: String::new(),
            model: None,
            stop_reason: None,
            finished: false,
            saw_done: false,
            saw_event: false,
            trace: None,
        }
    }

    /// Record usage a cut-short reply reported on `trace` (#2249 review).
    pub(crate) fn with_trace(
        mut self,
        trace: Option<std::sync::Arc<crate::domain::request_observation::RequestTrace>>,
    ) -> Self {
        self.trace = trace;
        self
    }

    pub(crate) fn with_model(model: impl Into<String>) -> Self {
        let mut handler = Self::new();
        handler.model = Some(model.into());
        handler
    }

    fn take_response(&mut self) -> LlmResponse {
        let content = if self.content.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.content))
        };
        let thinking_blocks = if self.reasoning.is_empty() {
            Vec::new()
        } else {
            vec![ThinkingBlock::Normal {
                thinking: std::mem::take(&mut self.reasoning),
                signature: String::new(),
            }]
        };
        let mut response = LlmResponse {
            content,
            tool_calls: std::mem::take(&mut self.tool_calls),
            usage: self.usage.take(),
            stop_reason: self.stop_reason.take(),
            thinking_blocks,
        };
        if let Some(model) = &self.model {
            crate::domain::usage_accounting::attach_cost(&mut response, model);
        }
        response
    }
}

impl SseHandler for OpenAiSseHandler {
    async fn process_line(
        &mut self,
        line: &str,
        tx: &tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> SseLineOutcome {
        let Some(data) = crate::infrastructure::providers::sse_common::event_data(line) else {
            return SseLineOutcome::Continue;
        };
        self.saw_event = true;
        if crate::infrastructure::providers::sse_end::is_done_marker(data) {
            self.saw_done = true;
            let _ = tx.send(StreamEvent::Done(self.take_response())).await;
            return SseLineOutcome::Done;
        }
        if let Ok(chunk) = serde_json::from_str::<serde_json::Value>(data) {
            // A mid-stream error chunk ends the stream as an error (#2236),
            // on this path too, which runs without admission: never skipped,
            // or end of file would send the partial reply as a whole one.
            if crate::infrastructure::providers::attempt_profile::is_stream_error_chunk(&chunk) {
                let message =
                    crate::infrastructure::providers::attempt_profile::openai_stream_error(&chunk);
                let _ = tx.send(StreamEvent::Error(message)).await;
                return SseLineOutcome::Done;
            }
            // Final usage chunk (requested via stream_options.include_usage).
            // Emitted with an empty `choices` array and a populated `usage`.
            if let Some(usage) = chunk.get("usage").and_then(|u| u.as_object()) {
                self.usage = Some(crate::infrastructure::providers::usage::parse_openai_usage(
                    usage,
                ));
            }
            if let Some(choices) = chunk.get("choices").and_then(|v| v.as_array()) {
                // Content is accumulated directly into `self.content` above, so
                // `apply_delta` only needs a throwaway sink for its own content
                // extraction. Reuse one buffer across choices instead of
                // allocating a fresh `String` per delta.
                for choice in choices {
                    if let Some(reason) = super::openai_sse_parser::choice_stop_reason(choice) {
                        self.stop_reason = Some(reason);
                    }
                    self.finished |= super::openai_sse_parser::is_finishing_choice(choice);
                    let delta = choice.get("delta").unwrap_or(&serde_json::Value::Null);
                    if let Some(text) = delta
                        .get("reasoning")
                        .or_else(|| delta.get("reasoning_content"))
                        .and_then(|v| v.as_str())
                    {
                        if let Err(err) = append_visible_thinking(
                            &mut self.reasoning,
                            text,
                            "OpenAI SSE reasoning",
                        ) {
                            let _ = tx.send(StreamEvent::Error(err.to_string())).await;
                            return SseLineOutcome::Done;
                        }
                        let _ = tx.send(StreamEvent::ThinkingDelta(text.to_string())).await;
                    }
                    if let Some(text) = delta.get("content").and_then(|v| v.as_str()) {
                        if let Err(err) = append_with_limit(
                            &mut self.content,
                            text,
                            MAX_OPENAI_SSE_CONTENT_BYTES,
                            "assistant content",
                        ) {
                            let _ = tx.send(StreamEvent::Error(err.to_string())).await;
                            return SseLineOutcome::Done;
                        }
                        let _ = tx.send(StreamEvent::TextDelta(text.to_string())).await;
                    }
                    self.delta_scratch.clear();
                    if let Err(err) = OpenAiProvider::apply_delta(
                        delta,
                        &mut self.delta_scratch,
                        &mut self.tool_calls,
                    ) {
                        let _ = tx.send(StreamEvent::Error(err.to_string())).await;
                        return SseLineOutcome::Done;
                    }
                }
            }
        }
        SseLineOutcome::Continue
    }

    async fn on_eof(&mut self, tx: &tokio::sync::mpsc::Sender<StreamEvent>) {
        // The pump ends the body here only when no line ended the stream,
        // so `[DONE]` never came (#2236).
        assert!(
            !self.saw_done,
            "an OpenAI stream ends at end of file only without [DONE]"
        );
        // Only a terminal signal ends a reply whole: a chunk naming its
        // finish reason is one. Without it, whatever output came, the reply
        // was cut short, or, with no event at all, the stream was empty.
        if self.finished {
            let _ = tx.send(StreamEvent::Done(self.take_response())).await;
        } else {
            // Tokens a usage chunk already reported were spent all the same.
            crate::infrastructure::providers::sse_end::record_unfinished_usage(
                self.trace.as_deref(),
                self.usage.take(),
                self.model.as_deref().unwrap_or_default(),
            );
            let error = crate::infrastructure::providers::sse_end::ended_early(
                self.saw_event,
                OPENAI_CUT_SHORT,
            );
            let _ = tx.send(StreamEvent::Error(error)).await;
        }
    }
}

/// Consume an OpenAI SSE byte stream, emitting `StreamEvent`s per delta.
#[cfg(test)]
pub(crate) async fn pump_sse_bytes(
    response: &mut reqwest::Response,
    tx: &tokio::sync::mpsc::Sender<StreamEvent>,
) {
    let mut handler = OpenAiSseHandler::new();
    crate::infrastructure::providers::sse_common::pump_sse(
        response,
        tx,
        &mut handler,
        Default::default(),
    )
    .await;
}

pub(crate) async fn pump_sse_bytes_for_model(
    response: &mut reqwest::Response,
    tx: &tokio::sync::mpsc::Sender<StreamEvent>,
    handler: OpenAiSseHandler,
    attempt: Option<super::super::attempt_transport::PassiveAttempt>,
    idle: super::super::stream_idle::StreamIdle,
) {
    super::super::attempt_transport::pump_observed(response, tx, handler, attempt, idle).await;
}

/// Consume an owned OpenAI SSE byte stream, emitting `StreamEvent`s per delta.
///
/// This is used by non-incremental `chat_stream`, which drains the event
/// receiver while this pump runs in a task. Keeping the pump concurrent with the
/// drain avoids deadlocking when a response has more deltas than the bounded
/// channel capacity.
pub(crate) async fn pump_sse_response_for_model(
    mut response: reqwest::Response,
    tx: tokio::sync::mpsc::Sender<StreamEvent>,
    handler: OpenAiSseHandler,
    attempt: Option<super::super::attempt_transport::PassiveAttempt>,
    idle: super::super::stream_idle::StreamIdle,
) {
    pump_sse_bytes_for_model(&mut response, &tx, handler, attempt, idle).await;
}

#[cfg(test)]
#[path = "openai_sse_end_tests.rs"]
mod end_tests;
#[cfg(test)]
#[path = "openai_finish_reason_tests.rs"]
mod finish_reason_tests;
#[cfg(test)]
#[path = "openai_sse_tests.rs"]
mod tests;
