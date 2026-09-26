//! Sending a request that may replay encrypted reasoning (#2162), and the
//! recovery when the service refuses it: the request is sent once more
//! without replay, and the provider replays no more — replay is an
//! optimisation, and a refused item must not fail every later turn.
use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::CodexProvider;
use super::codex_sse_handler::CodexSseHandler;
use crate::application::providers::ports::ChatRequest;
use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, Message, ThinkingBlock};
use crate::domain::provider::{CancelFlag, StreamEvent};
use crate::domain::request_observation::RequestTrace;
use crate::infrastructure::providers::attempt_profile::{Profile, Surface, Vendor};

/// Everything one request's sends share.
#[derive(Clone)]
pub(super) struct Call {
    pub(super) url: String,
    pub(super) session: Option<String>,
    pub(super) model: String,
    /// Stamped on the response's reasoning, even when this request
    /// replays none.
    pub(super) origin: String,
    pub(super) trace: Option<Arc<RequestTrace>>,
    pub(super) cancel: Option<CancelFlag>,
}

/// A request's body, and — when it replays reasoning — the same request
/// without it, sent if the first is refused.
pub(super) struct Bodies {
    pub(super) replaying: serde_json::Value,
    pub(super) plain: Option<serde_json::Value>,
}

impl CodexProvider {
    /// The call and bodies for `request`.
    pub(super) fn prepare(&self, request: &ChatRequest<'_>) -> (Call, Bodies) {
        let origin = self.reasoning_origin(request.model);
        let replay_to = match self.replay_refused.load(Ordering::Relaxed) {
            true => "",
            false => origin.as_str(),
        };
        let replaying = Self::build_request_body(request, &self.auth, replay_to);
        let plain = replays_any(request.messages, replay_to)
            .then(|| Self::build_request_body(request, &self.auth, ""));
        let call = Call {
            url: self.responses_url(),
            session: Self::request_session(request),
            model: request.model.to_string(),
            origin,
            trace: request.trace.clone(),
            cancel: request.cancel_flag.clone(),
        };
        (call, Bodies { replaying, plain })
    }

    /// Whether an error is the service refusing replayed reasoning.
    fn is_replay_refusal(message: &str) -> bool {
        let lower = message.to_ascii_lowercase();
        lower.contains("400") && lower.contains("encrypted")
    }

    fn note_replay_refused(&self, message: &str) {
        self.replay_refused.store(true, Ordering::Relaxed);
        tracing::warn!(
            error = %message,
            "Codex: replayed reasoning was refused; resent without it, and not replayed again"
        );
    }

    /// Send, and on a refused replay send once more without it.
    pub(super) async fn assemble(
        &self,
        call: &Call,
        bodies: Bodies,
    ) -> Result<LlmResponse, DomainError> {
        let first = self.assemble_once(call, &bodies.replaying).await;
        match (first, bodies.plain) {
            (Err(error), Some(plain)) if Self::is_replay_refusal(&error.to_string()) => {
                self.note_replay_refused(&error.to_string());
                self.assemble_once(call, &plain).await
            }
            (result, _) => result,
        }
    }

    async fn assemble_once(
        &self,
        call: &Call,
        body: &serde_json::Value,
    ) -> Result<LlmResponse, DomainError> {
        let builder = self
            .apply_headers(self.client.post(&call.url), call.session.as_deref())
            .json(body);
        if let Some(gate) = &self.attempt_admission {
            return super::super::attempt_transport::assembled(
                gate,
                call.trace.clone(),
                call.cancel.as_ref(),
                builder,
                Profile::new(Vendor::Codex, Surface::Assembled),
                |raw| {
                    let mut parsed = Self::parse_sse_response(raw)?;
                    Self::finish_response(&mut parsed, &call.model, &call.origin);
                    Ok(parsed)
                },
            )
            .await;
        }
        let resp = builder.send().await.map_err(|e| {
            DomainError::Provider(format!(
                "Codex request failed: {}",
                super::super::sse_common::format_send_error(&e)
            ))
        })?;
        let status = resp.status().as_u16();
        if status != 200 {
            let error_body = resp.text().await.unwrap_or_default();
            return Err(DomainError::Provider(format!(
                "HTTP {status} from Codex: {error_body}"
            )));
        }
        let raw = resp
            .text()
            .await
            .map_err(|e| DomainError::Provider(format!("failed to read response: {e}")))?;
        let mut parsed = Self::parse_sse_response(&raw)?;
        Self::finish_response(&mut parsed, &call.model, &call.origin);
        Ok(parsed)
    }

    /// Stream, and when the first event is a refused replay, stream once
    /// more without it; nothing of the refused attempt reaches `tx`.
    pub(super) async fn stream(
        self,
        call: Call,
        bodies: Bodies,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) {
        let Some(plain) = bodies.plain else {
            self.stream_once(&call, bodies.replaying, tx).await;
            return;
        };
        let (first_tx, mut first_rx) = tokio::sync::mpsc::channel(64);
        let attempt = {
            let provider = self.clone();
            let call = call.clone();
            tokio::spawn(async move {
                provider
                    .stream_once(&call, bodies.replaying, first_tx)
                    .await
            })
        };
        match first_rx.recv().await {
            Some(StreamEvent::Error(message)) if Self::is_replay_refusal(&message) => {
                let _ = attempt.await;
                self.note_replay_refused(&message);
                self.stream_once(&call, plain, tx).await;
            }
            Some(event) => {
                let mut next = Some(event);
                while let Some(event) = next {
                    if tx.send(event).await.is_err() {
                        break;
                    }
                    next = first_rx.recv().await;
                }
            }
            None => {}
        }
    }

    async fn stream_once(
        &self,
        call: &Call,
        body: serde_json::Value,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) {
        let handler = CodexSseHandler::with_model(&call.model, &call.origin);
        match &self.attempt_admission {
            Some(gate) => {
                let builder = self
                    .apply_headers(self.client.post(&call.url), call.session.as_deref())
                    .json(&body);
                super::super::attempt_transport::stream(
                    gate,
                    (call.trace.clone(), call.cancel.as_ref()),
                    builder,
                    Profile::new(Vendor::Codex, Surface::Incremental),
                    tx,
                    handler,
                )
                .await;
            }
            None => {
                self.pump_codex_sse(&call.url, body, tx, call.session.as_deref(), handler)
                    .await;
            }
        }
    }
}

/// Whether any message carries reasoning for `origin` (none for `""`).
fn replays_any(messages: &[Message], origin: &str) -> bool {
    !origin.is_empty()
        && messages.iter().any(|message| {
            message.thinking_blocks.iter().any(|block| {
                matches!(block, ThinkingBlock::EncryptedReasoning { origin: from, .. } if from == origin)
            })
        })
}
