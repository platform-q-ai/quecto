//! Instrumented pump preserves the common pump behavior while reporting transport exits.
use super::*;
use crate::infrastructure::providers::sse_common::line_within_limit;

impl Receipt {
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
) {
    let mut carry: Vec<u8> = Vec::new();

    loop {
        let bytes = match response.chunk().await {
            Ok(Some(b)) => b,
            Ok(None) => break,
            Err(e) => {
                receipt.termination(Termination::ReadError);
                let _ = tx
                    .send(StreamEvent::Error(format!("stream read error: {e}")))
                    .await;
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
        }
        // Guard against unbounded line growth from a misbehaving server.
        if !line_within_limit(carry.len()) {
            refuse_long_line(receipt, tx).await;
            return;
        }
    }

    // Clean EOF — let the handler finalize.
    handler.on_eof(tx).await;
}

/// The common pump's refusal of a line over the limit, recorded as how the
/// attempt ended.
async fn refuse_long_line(receipt: &Receipt, tx: &tokio::sync::mpsc::Sender<StreamEvent>) {
    {
        let mut state = receipt.0.lock().unwrap_or_else(|e| e.into_inner());
        state.diagnostics.oversized_lines = state.diagnostics.oversized_lines.saturating_add(1);
    }
    receipt.termination(Termination::ReadError);
    super::super::sse_common::refuse_long_line(tx).await;
}
