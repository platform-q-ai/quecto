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
use crate::infrastructure::providers::attempt_transport::PassiveAttempt;
use crate::infrastructure::providers::input_prefix::MeasuredInput;
use crate::infrastructure::providers::stream_idle::BodyError;

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
    /// The replaying body's input measured, kept as its session's baseline
    /// once a send is accepted (#2398); `None` when not observed.
    pub(super) measured: Option<MeasuredInput>,
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
        // Where the input sent first differs from its session's last
        // accepted request (#2398), compared as the first body sends it.
        // Only an observed request of a named session is compared, and it
        // becomes the baseline only once a send of it is accepted.
        let measured = match (&request.trace, request.session_id) {
            (Some(trace), Some(session)) => {
                let measured = MeasuredInput::of(session, &replaying);
                trace.record_input_prefix(self.input_digests.compare(&measured));
                Some(measured)
            }
            (None, _) | (_, None) => None,
        };
        let call = Call {
            url: self.responses_url(),
            session: Self::request_session(request),
            model: request.model.to_string(),
            origin,
            trace: request.trace.clone(),
            cancel: request.cancel_flag.clone(),
        };
        let bodies = Bodies {
            replaying,
            plain,
            measured,
        };
        (call, bodies)
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

    /// Send, and on a refused replay send once more without it; the body
    /// accepted becomes its session's baseline (#2398).
    pub(super) async fn assemble(
        &self,
        call: &Call,
        bodies: Bodies,
    ) -> Result<LlmResponse, DomainError> {
        let Bodies {
            replaying,
            plain,
            measured,
        } = bodies;
        let first = self.assemble_once(call, &replaying).await;
        let (result, accepted) = match (first, plain) {
            (Err(error), Some(plain)) if Self::is_replay_refusal(&error.to_string()) => {
                self.note_replay_refused(&error.to_string());
                let measured = measured.map(|replaying| replaying.for_body(&plain));
                (self.assemble_once(call, &plain).await, measured)
            }
            (result, _) => (result, measured),
        };
        if result.is_ok() {
            self.accept(accepted);
        }
        result
    }

    /// A send of the measured body was accepted (#2398).
    fn accept(&self, measured: Option<MeasuredInput>) {
        if let Some(measured) = measured {
            self.input_digests.commit(measured);
        }
    }

    /// Forward a send's events from `first` on; the first that is not an
    /// error shows the send accepted (#2398).
    async fn forward(
        &self,
        first: Option<StreamEvent>,
        events: &mut tokio::sync::mpsc::Receiver<StreamEvent>,
        tx: &tokio::sync::mpsc::Sender<StreamEvent>,
        mut measured: Option<MeasuredInput>,
    ) {
        let mut next = first;
        while let Some(event) = next {
            match &event {
                StreamEvent::TextDelta(_)
                | StreamEvent::ThinkingDelta(_)
                | StreamEvent::ToolCallStart { .. }
                | StreamEvent::ToolCallDelta(_)
                | StreamEvent::ToolCallEnd { .. }
                | StreamEvent::Done(_) => self.accept(measured.take()),
                StreamEvent::Error(_) => {}
            }
            if tx.send(event).await.is_err() {
                break;
            }
            next = next_event(events, tx).await;
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
                Profile::new(Vendor::Codex, Surface::Assembled, self.stream_idle),
                |raw| {
                    let mut parsed = Self::parse_sse_reply(raw)
                        .map_err(|cut| cut.account(call.trace.as_deref(), &call.model))?;
                    Self::finish_response(&mut parsed, &call.model, &call.origin);
                    Ok(parsed)
                },
            )
            .await;
        }
        // Observed beside the request, never altering it (#2151, #2210).
        let idle = self.stream_idle;
        let profile = Profile::new(Vendor::Codex, Surface::Assembled, idle);
        let attempt = PassiveAttempt::begin(call.trace.clone(), profile);
        let resp = match idle.within(builder.send()).await {
            Ok(sent) => sent
                .inspect_err(|_| attempt.iter().for_each(|a| a.send_failed()))
                .map_err(|e| {
                    DomainError::Provider(format!(
                        "Codex request failed: {}",
                        super::super::sse_common::format_send_error(&e)
                    ))
                })?,
            Err(silent) => {
                attempt.iter().for_each(|a| a.idle());
                return Err(DomainError::Provider(silent.to_string()));
            }
        };
        if let Some(attempt) = &attempt {
            attempt.response(&resp);
        }
        let status = resp.status().as_u16();
        if status != 200 {
            let read = idle.text(resp).await;
            attempt.iter().for_each(|a| a.error_read(status, &read));
            let error_body = crate::infrastructure::providers::stream_idle::error_text(read);
            return Err(DomainError::Provider(format!(
                "HTTP {status} from Codex: {error_body}"
            )));
        }
        let raw = match &attempt {
            Some(attempt) => attempt.read_sse(resp, profile).await?,
            None => idle.text(resp).await.map_err(|e| match e {
                BodyError::Idle(silent) => DomainError::Provider(silent.to_string()),
                BodyError::Read(e) => {
                    DomainError::Provider(format!("failed to read response: {e}"))
                }
            })?,
        };
        let parsed = Self::parse_sse_reply(&raw)
            .map_err(|cut| cut.account(call.trace.as_deref(), &call.model));
        attempt.iter().for_each(|a| a.parsed(&parsed));
        let mut parsed = parsed?;
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
        let Bodies {
            replaying,
            plain,
            measured,
        } = bodies;
        let (first_tx, mut first_rx) = tokio::sync::mpsc::channel(64);
        let attempt = {
            let provider = self.clone();
            let call = call.clone();
            tokio::spawn(async move { provider.stream_once(&call, replaying, first_tx).await })
        };
        let first = next_event(&mut first_rx, &tx).await;
        match (first, plain) {
            (Some(StreamEvent::Error(message)), Some(plain))
                if Self::is_replay_refusal(&message) =>
            {
                let _ = attempt.await;
                self.note_replay_refused(&message);
                let measured = measured.map(|replaying| replaying.for_body(&plain));
                let (plain_tx, mut plain_rx) = tokio::sync::mpsc::channel(64);
                let (this, tx) = (&self, &tx);
                // Owns its receiver: returning drops it, which cancels the
                // send once the caller has gone.
                let forward = async move {
                    let first = next_event(&mut plain_rx, tx).await;
                    this.forward(first, &mut plain_rx, tx, measured).await;
                };
                tokio::join!(self.stream_once(&call, plain, plain_tx), forward);
            }
            (first, _) => self.forward(first, &mut first_rx, &tx, measured).await,
        }
    }

    async fn stream_once(
        &self,
        call: &Call,
        body: serde_json::Value,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) {
        let handler =
            CodexSseHandler::with_model(&call.model, &call.origin).with_trace(call.trace.clone());
        match &self.attempt_admission {
            Some(gate) => {
                let builder = self
                    .apply_headers(self.client.post(&call.url), call.session.as_deref())
                    .json(&body);
                super::super::attempt_transport::stream(
                    gate,
                    (call.trace.clone(), call.cancel.as_ref()),
                    builder,
                    Profile::new(Vendor::Codex, Surface::Incremental, self.stream_idle),
                    tx,
                    handler,
                )
                .await;
            }
            None => self.pump_codex_sse(call, body, tx, handler).await,
        }
    }
}

/// The next event of a send, or `None` once the caller has gone: a send's
/// transport stops when its receiver is dropped, so a caller that drops its
/// own must not leave the send blocked behind this relay.
async fn next_event(
    events: &mut tokio::sync::mpsc::Receiver<StreamEvent>,
    tx: &tokio::sync::mpsc::Sender<StreamEvent>,
) -> Option<StreamEvent> {
    tokio::select! {
        event = events.recv() => event,
        () = tx.closed() => None,
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
