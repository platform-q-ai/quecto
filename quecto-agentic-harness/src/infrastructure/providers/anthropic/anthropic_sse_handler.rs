//! The Anthropic Messages SSE line handler: the incremental read of a
//! reply, which ends whole only at `message_stop` (#2249 review).
use std::sync::Arc;

use super::{SseAccumulator, dispatch_sse_event};
use crate::domain::inference::events::request_observation::RequestTrace;
use crate::domain::inference::value_objects::provider::StreamEvent;
use crate::domain::tool_policy::value_objects::tool::ToolDefinition;
use crate::infrastructure::providers::sse_common::{SseHandler, SseLineOutcome};
use crate::infrastructure::providers::sse_end;

/// SSE line handler for the Anthropic Messages API.
pub(crate) struct AnthropicSseHandler {
    current_event: String,
    acc: SseAccumulator,
    saw_terminal: bool,
    /// Whether any event came: a body with none is an empty stream.
    saw_event: bool,
    model: Option<String>,
    /// Where usage reported before a cut is recorded (#2249 review).
    trace: Option<Arc<RequestTrace>>,
}

impl AnthropicSseHandler {
    pub(in crate::infrastructure::providers::anthropic) fn new(
        tool_defs: Option<Vec<ToolDefinition>>,
    ) -> Self {
        Self {
            current_event: String::new(),
            acc: match tool_defs {
                Some(defs) => SseAccumulator::with_tool_defs(defs),
                None => SseAccumulator::default(),
            },
            saw_terminal: false,
            saw_event: false,
            model: None,
            trace: None,
        }
    }

    pub(crate) fn with_model(tool_defs: Option<Vec<ToolDefinition>>, model: &str) -> Self {
        let mut handler = Self::new(tool_defs);
        handler.model = Some(model.to_string());
        handler
    }

    /// Record usage a cut-short reply reported on `trace` (#2249 review).
    pub(crate) fn with_trace(mut self, trace: Option<Arc<RequestTrace>>) -> Self {
        self.trace = trace;
        self
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn new_for_test(tool_defs: Option<Vec<ToolDefinition>>) -> Self {
        Self::new(tool_defs)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(in crate::infrastructure::providers::anthropic) fn into_response(
        self,
    ) -> crate::domain::conversation::value_objects::message::LlmResponse {
        self.acc.into_response()
    }
}

impl SseHandler for AnthropicSseHandler {
    async fn process_line(
        &mut self,
        line: &str,
        tx: &tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> SseLineOutcome {
        let data = crate::infrastructure::providers::sse_common::event_data(line);
        if line.starts_with("event: ") || data.is_some() {
            self.saw_event = true;
        }
        if let Some(event_type) = line.strip_prefix("event: ") {
            self.current_event = event_type.to_string();
        } else if let Some(data) = data {
            let chunk_val: serde_json::Value = serde_json::from_str(data).unwrap_or_default();
            if dispatch_sse_event(
                &self.current_event,
                &chunk_val,
                &mut self.acc,
                self.model.as_deref(),
                tx,
            )
            .await
            {
                self.saw_terminal = true;
                return SseLineOutcome::Done;
            }
        }
        SseLineOutcome::Continue
    }

    async fn on_eof(&mut self, tx: &tokio::sync::mpsc::Sender<StreamEvent>) {
        // The pump ends the body here only when no line ended the stream,
        // so `message_stop` never came: whatever output came, the reply was
        // cut short, or, with no event at all, the stream was empty (#2249
        // review). Tokens it reported (`message_start`) were spent all the
        // same, so they are recorded for accounting.
        assert!(
            !self.saw_terminal,
            "an Anthropic stream ends at end of file only without message_stop"
        );
        let usage = std::mem::take(&mut self.acc).into_response().usage;
        sse_end::record_unfinished_usage(
            self.trace.as_deref(),
            usage,
            self.model.as_deref().unwrap_or_default(),
        );
        let error = sse_end::ended_early(self.saw_event, ANTHROPIC_CUT_SHORT);
        let _ = tx.send(StreamEvent::Error(error)).await;
    }
}

/// The error an Anthropic body ends with when it ends before `message_stop`,
/// output or not (#2249 review): a reply cut short, worded as the transport
/// cut it is, so the retry classifier reads it as a retryable network
/// failure.
pub(crate) const ANTHROPIC_CUT_SHORT: &str =
    "Anthropic stream ended without completion: connection closed before message_stop";
