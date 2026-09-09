//! Instrumented pump preserves the common pump behavior while reporting transport exits.
use super::*;
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

        // Guard against unbounded line growth from a misbehaving server.
        if carry.len().saturating_add(bytes.len()) <= super::super::sse_common::MAX_SSE_LINE_BYTES
            || carry.contains(&b'\n')
        {
            carry.extend_from_slice(&bytes);
        } else {
            {
                let mut state = receipt.0.lock().unwrap();
                state.diagnostics.oversized_lines =
                    state.diagnostics.oversized_lines.saturating_add(1);
            }
            receipt.termination(Termination::ReadError);
            let _ = tx
                .send(StreamEvent::Error("SSE line exceeds 1 MiB limit".into()))
                .await;
            return;
        }

        // Drain complete lines — decode in-place to avoid per-line allocation.
        while let Some(pos) = carry.iter().position(|&b| b == b'\n') {
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
    }

    // Clean EOF — let the handler finalize.
    handler.on_eof(tx).await;
}
