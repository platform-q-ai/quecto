//! Bounded message reader task for the UDS command loop.
//!
//! Extracted from `uds.rs` (#1003) to keep that module under the
//! per-file line-count gate. Since #1059 (ADR-0008 part 1) the reader speaks
//! the deprecation-window protocol: each incoming message is sniffed as a
//! length-prefixed frame or a legacy NDJSON line
//! (`quecto_line_io::read_frame_or_legacy_line`), the detected framing is
//! recorded on the shared [`ConnectionWireMode`] so replies use the same
//! framing, and protocol violations are surfaced explicitly instead of
//! misparsed or buffered unbounded:
//!
//! - an over-cap frame/line is rejected while the connection stays usable
//!   ([`Violation::Oversized`], which also fails the client's oldest
//!   pending extension call, #2423, then keep reading);
//! - a peer speaking neither framing is an explicit version mismatch
//!   ([`Violation::ProtocolError`] then EOF — never a hang).

use super::uds::MAX_FRAME_PAYLOAD_BYTES;
use super::uds::{is_abort_command, steer_images};
use super::uds_cancel::{CancelHandle, TurnControlHandle, fire_cancel};
use super::uds_wire::ConnectionWireMode;
use quecto_line_io::{FrameError, Incoming, WireMode};
use tokio::io::BufReader;
use tokio::sync::mpsc;

/// A message delivered from the reader task to the command loop.
pub(super) enum ReaderMessage {
    /// A complete message within the byte cap, with the images the reader
    /// admitted for a steer (#2422), run by dispatch without decoding again.
    Message(String, Option<super::uds::AdmittedImages>),
    /// A protocol violation to surface to the client as an error event.
    Violation(Violation),
}

/// A protocol violation the reader met.
pub(super) enum Violation {
    /// A version mismatch (the reader closes right after).
    ProtocolError(String),
    /// An over-cap frame/line, dropped unread (recoverable — more messages
    /// may follow): its error text and declared size (#2423 review M2).
    Oversized((String, usize)),
}

/// Tell the client about a protocol violation its reader met: a
/// `protocol_error` response. A message over the cap may have been an
/// extension's result (#2423 review M2), so the client's oldest pending
/// call fails now, not at its timeout.
pub(super) async fn report_protocol_error(
    ctx: &mut super::uds::DispatchCtx<'_>,
    violation: Violation,
) {
    let message = match violation {
        Violation::ProtocolError(message) => message,
        Violation::Oversized(over_cap) => super::uds_ext_protocol::reject_oversized(
            &ctx.client_tool_registry,
            ctx.current_client_id,
            over_cap,
        ),
    };
    tracing::warn!("UDS protocol error: {message}");
    let event = super::protocol::AgentEvent::err(None, "protocol_error", message);
    super::uds::emit_event_to_broadcast_or_writer(ctx, &event).await;
}

/// Spawn the bounded reader task. Returns its [`JoinHandle`] (for abort on
/// loop exit) and the receiving end of the channel.
pub(super) fn spawn_reader_task(
    reader: Box<dyn tokio::io::AsyncRead + Send + Unpin>,
    cancel_for_reader: CancelHandle,
    control_for_reader: TurnControlHandle,
    wire_mode: ConnectionWireMode,
) -> (
    tokio::task::JoinHandle<()>,
    mpsc::Receiver<Option<ReaderMessage>>,
) {
    let (tx, rx) = mpsc::channel::<Option<ReaderMessage>>(64);

    let handle = tokio::spawn(async move {
        let mut reader = BufReader::new(reader);
        loop {
            match quecto_line_io::read_frame_or_legacy_line(&mut reader, MAX_FRAME_PAYLOAD_BYTES)
                .await
            {
                Ok(Some(incoming)) => {
                    let (mode, bytes) = match incoming {
                        Incoming::Frame(b) => (WireMode::Framed, b),
                        Incoming::LegacyLine(b) => (WireMode::LegacyLine, b),
                    };
                    // Record the peer's framing so replies use it too (#1059).
                    wire_mode.record(mode);
                    // Reuse the payload `Vec`'s allocation on the common
                    // valid-UTF-8 path; only pay a copy for the lossy fallback
                    // on malformed input (preserving the tolerate-non-UTF-8
                    // behaviour).
                    let line = String::from_utf8(bytes)
                        .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
                    let trimmed = line.trim();
                    // A steer's images are admitted once, here (#2422).
                    let steer = steer_images(trimmed);
                    // Record operator intent BEFORE firing the cancel so the
                    // post-cancel idle drain cannot observe the cancel and
                    // run a nudge before the abort/steer flag lands (#895/#896).
                    if is_abort_command(trimmed) {
                        control_for_reader.mark_abort();
                        fire_cancel(&cancel_for_reader);
                    } else if steer.is_some() {
                        control_for_reader.mark_steer();
                        fire_cancel(&cancel_for_reader);
                    }
                    let message = ReaderMessage::Message(line, steer);
                    if tx.send(Some(message)).await.is_err() {
                        break;
                    }
                }
                Err(err @ FrameError::Oversized { declared, .. }) => {
                    // Clean rejection: the declared payload was consumed, so
                    // subsequent frames on this connection still parse.
                    let over_cap = (err.to_string(), declared);
                    if tx
                        .send(Some(ReaderMessage::Violation(Violation::Oversized(
                            over_cap,
                        ))))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Err(err @ FrameError::VersionMismatch { .. }) => {
                    // Explicit, loggable failure — never silent misparsing or
                    // a hang. The connection is unusable; close after
                    // surfacing the error.
                    let _ = tx
                        .send(Some(ReaderMessage::Violation(Violation::ProtocolError(
                            err.to_string(),
                        ))))
                        .await;
                    let _ = tx.send(None).await;
                    break;
                }
                _ => {
                    let _ = tx.send(None).await;
                    break;
                }
            }
        }
    });

    (handle, rx)
}
