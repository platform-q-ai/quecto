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
//!   ([`RawLine::ProtocolError`] then keep reading);
//! - a peer speaking neither framing is an explicit version mismatch
//!   ([`RawLine::ProtocolError`] then EOF — never a hang).

use super::uds::MAX_LINE_BYTES;
use super::uds::{is_abort_command, is_cancel_command, is_steer_command};
use super::uds_cancel::{CancelHandle, TurnControlHandle, fire_cancel};
use super::uds_wire::ConnectionWireMode;
use quecto_line_io::{FrameError, Incoming, WireMode};
use tokio::io::BufReader;
use tokio::sync::mpsc;

/// A message delivered from the reader task to the command loop.
pub(super) enum RawLine {
    /// A complete message within the byte cap.
    Line(String),
    /// A protocol violation to surface to the client as an error event: an
    /// over-cap frame/line (recoverable — more messages may follow) or a
    /// version mismatch (the reader closes right after).
    ProtocolError(String),
}

/// Spawn the bounded reader task. Returns its [`JoinHandle`] (for abort on
/// loop exit) and the receiving end of the channel.
pub(super) fn spawn_reader_task(
    reader: Box<dyn tokio::io::AsyncRead + Send + Unpin>,
    cancel_for_reader: CancelHandle,
    control_for_reader: TurnControlHandle,
    wire_mode: ConnectionWireMode,
) -> (tokio::task::JoinHandle<()>, mpsc::Receiver<Option<RawLine>>) {
    let (tx, rx) = mpsc::channel::<Option<RawLine>>(64);

    let handle = tokio::spawn(async move {
        let mut reader = BufReader::new(reader);
        loop {
            match quecto_line_io::read_frame_or_legacy_line(&mut reader, MAX_LINE_BYTES).await {
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
                    if is_cancel_command(trimmed) {
                        // Record operator intent BEFORE firing the cancel so the
                        // post-cancel idle drain cannot observe the cancel and
                        // run a nudge before the abort/steer flag lands (#895/#896).
                        if is_abort_command(trimmed) {
                            control_for_reader.mark_abort();
                        } else if is_steer_command(trimmed) {
                            control_for_reader.mark_steer();
                        }
                        fire_cancel(&cancel_for_reader);
                    }
                    if tx.send(Some(RawLine::Line(line))).await.is_err() {
                        break;
                    }
                }
                Err(err @ FrameError::Oversized { .. }) => {
                    // Clean rejection: the declared payload was consumed, so
                    // subsequent frames on this connection still parse.
                    if tx
                        .send(Some(RawLine::ProtocolError(err.to_string())))
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
                    let _ = tx.send(Some(RawLine::ProtocolError(err.to_string()))).await;
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
