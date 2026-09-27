//! Instrumented pump preserves the common pump behavior while reporting transport exits;
//! so do the send and the error body read of an owned attempt.
use super::*;
use crate::infrastructure::providers::sse_common::line_within_limit;
use crate::infrastructure::providers::stream_idle::StreamIdle;

impl Receipt {
    /// A whole reply was read: it ended as `read` when accepted, rejected
    /// when it could not be (#2156 review).
    pub(super) fn accepted<T>(
        &self,
        parsed: Result<T, DomainError>,
        read: Termination,
    ) -> Result<T, DomainError> {
        self.termination(match parsed {
            Ok(_) => read,
            Err(_) => Termination::Rejected,
        });
        parsed
    }

    /// The provider went silent for the whole idle bound (#2210): the
    /// attempt ends as idle, failing with the idle error.
    pub(super) fn idle(&self, silent: super::super::stream_idle::Idle) -> DomainError {
        self.termination(Termination::Idle);
        DomainError::Provider(silent.to_string())
    }

    /// A whole non-streaming reply took the total bound (#2210 review): the
    /// attempt ends as timed out, failing with the timeout error.
    pub(super) fn timed_out(&self, late: super::super::stream_idle::TimedOut) -> DomainError {
        self.termination(Termination::TimedOut);
        DomainError::Provider(late.to_string())
    }

    /// The attempt's output passed its cap (#2210): it fails, ends as
    /// output-capped, with the cap error's message.
    pub(super) fn output_capped(
        &self,
        capped: crate::domain::request_progress::OutputCapped,
    ) -> String {
        self.fail();
        self.termination(Termination::OutputCapped);
        capped.to_string()
    }

    /// The handler ended the stream: a terminal event already recorded how,
    /// so only an ending no event explains is the harness refusing the reply
    /// (#2156 review).
    pub(super) fn refused(&self) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.diagnostics.termination == Termination::Dropped {
            state.diagnostics.termination = Termination::Rejected;
        }
    }
}
pub(super) async fn pump_sse<H: SseHandler>(
    receipt: &Receipt,
    response: &mut reqwest::Response,
    tx: &tokio::sync::mpsc::Sender<StreamEvent>,
    handler: &mut H,
    idle: StreamIdle,
) {
    let mut carry: Vec<u8> = Vec::new();

    loop {
        let bytes = match idle.within(response.chunk()).await {
            Ok(Ok(Some(b))) => b,
            Ok(Ok(None)) => break,
            Ok(Err(e)) => {
                receipt.termination(Termination::ReadError);
                let _ = tx
                    .send(StreamEvent::Error(format!("stream read error: {e}")))
                    .await;
                return;
            }
            Err(silent) => {
                receipt.termination(Termination::Idle);
                let _ = tx.send(StreamEvent::Error(silent.to_string())).await;
                return;
            }
        };

        carry.extend_from_slice(&bytes);

        // Drain complete lines — decode in-place to avoid per-line allocation.
        // Each line is held to the limit on its own, as the common pump does.
        while let Some(pos) = carry.iter().position(|&b| b == b'\n') {
            if !line_within_limit(pos) {
                refuse_long_line(receipt, tx).await;
                return;
            }
            let done = if let Ok(line) = std::str::from_utf8(&carry[..=pos]) {
                let line = line.trim_end_matches(['\n', '\r']);
                matches!(handler.process_line(line, tx).await, SseLineOutcome::Done)
            } else {
                {
                    let mut state = receipt.0.lock().unwrap();
                    state.diagnostics.parse_errors =
                        state.diagnostics.parse_errors.saturating_add(1);
                }
                false
            };
            carry.drain(..=pos);
            if done {
                return;
            }
            if let Some(capped) = receipt.capped() {
                let error = receipt.output_capped(capped);
                let _ = tx.send(StreamEvent::Error(error)).await;
                return;
            }
        }
        // Guard against unbounded line growth from a misbehaving server.
        if !line_within_limit(carry.len()) {
            refuse_long_line(receipt, tx).await;
            return;
        }
    }

    // Clean EOF — let the handler finalize. A last line with no newline
    // is never handed to the handler, observed or not, so no output past
    // the cap is delivered from it (#2210 review).
    handler.on_eof(tx).await;
}

/// The common pump's refusal of a line over the limit, recorded as how the
/// attempt ended: a limit the harness enforces, so rejected.
async fn refuse_long_line(receipt: &Receipt, tx: &tokio::sync::mpsc::Sender<StreamEvent>) {
    {
        let mut state = receipt.0.lock().unwrap_or_else(|e| e.into_inner());
        state.diagnostics.oversized_lines = state.diagnostics.oversized_lines.saturating_add(1);
    }
    receipt.termination(Termination::Rejected);
    super::super::sse_common::refuse_long_line(tx).await;
}

pub(super) async fn send(
    builder: reqwest::RequestBuilder,
    receipt: &Receipt,
    profile: Profile,
) -> Result<reqwest::Response, DomainError> {
    let response = match profile.within(builder.send()).await {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => {
            receipt.termination(Termination::SendError);
            return Err(profile.send_error(&error));
        }
        Err(silent) => return Err(receipt.idle(silent)),
    };
    receipt.headers(&response);
    Ok(response)
}
pub(super) async fn error_body(
    response: reqwest::Response,
    receipt: &Receipt,
    profile: Profile,
) -> DomainError {
    use super::super::stream_idle::BodyError;
    let status = response.status().as_u16();
    let suffix = profile.suffix(response.headers());
    let text = match profile.text(response).await {
        Ok(text) => text,
        Err(BodyError::Read(error)) if profile.strict_error_body() => {
            receipt.termination(Termination::ReadError);
            return profile.read_error(&error);
        }
        Err(BodyError::Read(_)) => {
            receipt.termination(Termination::ReadError);
            String::new()
        }
        Err(BodyError::Idle(idle)) => {
            receipt.termination(Termination::Idle);
            idle.body_marker()
        }
    };
    if receipt.0.lock().unwrap().diagnostics.termination == Termination::Dropped {
        receipt.termination(Termination::HttpError);
    }
    receipt.http_error(status, &text);
    let text = profile.error_body(text);
    DomainError::Provider(format!(
        "HTTP {status} from {}: {text}{suffix}",
        profile.name()
    ))
}
