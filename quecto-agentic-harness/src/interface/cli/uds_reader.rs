//! Bounded line reader task for the UDS command loop.
//!
//! Extracted from `uds.rs` (#1003) to keep that module under the
//! per-file line-count gate. The reader enforces the 1 MiB line cap *while
//! reading* via `quecto_line_io::read_bounded_line`, surfacing oversized lines
//! as [`RawLine::TooLong`] instead of buffering them in full.

use super::uds::MAX_LINE_BYTES;
use super::uds::{is_abort_command, is_cancel_command, is_steer_command};
use super::uds_cancel::{CancelHandle, TurnControlHandle, fire_cancel};
use tokio::io::BufReader;
use tokio::sync::mpsc;

/// A line delivered from the reader task to the command loop.
pub(super) enum RawLine {
    /// A complete line within the byte cap.
    Line(String),
    /// A line that exceeded [`MAX_LINE_BYTES`] and was capped while reading.
    TooLong,
}

/// Spawn the bounded reader task. Returns its [`JoinHandle`] (for abort on
/// loop exit) and the receiving end of the channel.
pub(super) fn spawn_reader_task(
    reader: Box<dyn tokio::io::AsyncRead + Send + Unpin>,
    cancel_for_reader: CancelHandle,
    control_for_reader: TurnControlHandle,
) -> (tokio::task::JoinHandle<()>, mpsc::Receiver<Option<RawLine>>) {
    let (tx, rx) = mpsc::channel::<Option<RawLine>>(64);

    let handle = tokio::spawn(async move {
        let mut reader = BufReader::new(reader);
        loop {
            match quecto_line_io::read_bounded_line(&mut reader, MAX_LINE_BYTES).await {
                Ok(Some(bounded)) => {
                    if bounded.truncated {
                        // Surface a "line too long" parse error to the caller
                        // instead of silently dropping the (now-capped) line,
                        // matching the pre-#1003 `parse_line` behavior — but the
                        // cap is now enforced *while reading*, not after fully
                        // buffering the oversized line (#1003).
                        if tx.send(Some(RawLine::TooLong)).await.is_err() {
                            break;
                        }
                        continue;
                    }
                    let line = bounded.content;
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
                _ => {
                    let _ = tx.send(None).await;
                    break;
                }
            }
        }
    });

    (handle, rx)
}
