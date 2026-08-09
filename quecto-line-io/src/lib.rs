//! Shared bounded reader for quecto's framed JSON UDS protocol.
//!
//! Several crates (`quecto-api`, `quecto-agentic-harness`) read
//! length-prefixed JSON frames (or legacy `\n`-terminated JSON lines) from a `UnixStream`/pipe
//! treated as untrusted (a UDS client, a spawned sub-agent, or a parent
//! process). Reading with `AsyncBufReadExt::lines()`/`read_line` buffers the
//! *entire* line before any length check can run, so one giant unterminated
//! line can grow the buffer without bound before it's dropped.
//!
//! `quecto-tui` uses [`read_bounded_line_into`] so its hot event-reader path
//! can reuse a caller-owned buffer (with `shrink_to` reclaim) without keeping a
//! separate copy of the same capped framing logic.
//!
//! [`read_bounded_line`] caps memory growth *while* reading: once the
//! accumulated line reaches `max_bytes`, any further bytes up to the next
//! `\n` are read from the socket (so the stream stays framed) but discarded
//! rather than appended, so the buffer never exceeds `max_bytes`. Callers
//! that want the exceeded case to be a hard read error (e.g. a required
//! response) can check [`BoundedLine::truncated`] and translate it into their
//! own error type.

use tokio::io::{AsyncBufRead, AsyncBufReadExt};

/// Single source of truth for quecto's UDS protocol payload cap
/// (8 MiB, INCLUDING the trailing `\n`).
///
/// Every reader bound and emitter cap in the workspace derives from this
/// constant (`quecto-tui`'s `MAX_LINE_BYTES`, the harness's
/// `EVENT_LINE_CAP_BYTES` and UDS/sub-agent read bounds, `quecto-api`'s
/// client bound), so an emitter can never legally produce a line a reader
/// drops unread (#1047). Hand-pinning the value in a dependent crate instead
/// of referencing this constant reintroduces that failure mode.
///
/// The bound stays finite (readers still reject larger frames before buffering,
/// preserving ADR-0008's anti-OOM guarantee). Large message recovery is handled
/// by existing ranged `get_message` requests in consumer crates; this crate does
/// not define a new chunking format or a different cap for that path.
pub const PROTOCOL_LINE_CAP_BYTES: usize = 8 * 1_048_576;

/// A single `\n`-terminated (or EOF-terminated final) line, capped to at most
/// `max_bytes` of content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedLine {
    /// The line's content, excluding the trailing `\n`, capped to `max_bytes`.
    ///
    /// Truncation is *byte-wise* and may split a multi-byte UTF-8 codepoint;
    /// in that case the lossy conversion replaces the dangling prefix with
    /// U+FFFD (3 bytes), so on that path `content.len()` may slightly exceed
    /// `max_bytes`.
    pub content: String,
    /// `true` if the source line on the wire was longer than `max_bytes` and
    /// bytes beyond the cap were discarded (rather than buffered).
    pub truncated: bool,
}

/// Metadata for a line read into a caller-owned byte buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundedLineRead {
    /// Number of bytes consumed from the underlying reader, including a
    /// terminating `\n` when one was present and including bytes discarded after
    /// `max_bytes` for oversized lines.
    pub bytes_read: usize,
    /// `true` if the wire line exceeded `max_bytes` bytes and the tail was
    /// consumed but not appended to the caller's buffer.
    pub truncated: bool,
}

const INITIAL_CAPACITY: usize = 8 * 1024;
const RECLAIM_THRESHOLD: usize = 64 * 1024;

/// Convert the accumulated line bytes into a `String`, reusing the existing
/// `Vec` allocation on the common valid-UTF-8 path and only paying a lossy
/// re-allocation when the bytes are not valid UTF-8.
fn finish(buf: Vec<u8>) -> String {
    match String::from_utf8(buf) {
        Ok(s) => s,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    }
}

/// Read one line from `reader`, capping buffered content to `max_bytes`.
///
/// Returns `Ok(None)` at EOF with no trailing partial data. Returns
/// `Ok(Some(BoundedLine { truncated: true, .. }))` when the line on the wire
/// exceeded `max_bytes` — the stream is still fully drained up to (and
/// including) the next `\n`, so framing is preserved for the next call.
///
/// This allocate-per-call convenience wrapper is implemented on top of
/// [`read_bounded_line_into`]. Use that lower-level API when a hot loop should
/// reuse a caller-owned `Vec<u8>` or distinguish invalid UTF-8 from valid text.
///
/// # Caveats
/// - **Not cancellation-safe.** If the returned future is dropped before
///   completion (e.g. losing a `select!` race), bytes already consumed from
///   `reader` for the partially-read line are lost; the next call starts
///   mid-line.
/// - A trailing `\r` is **preserved** (unlike `AsyncBufReadExt::lines`, which
///   strips `\r\n`). Callers speaking a `\r\n`-tolerant protocol must trim it
///   themselves.
/// - Truncation is byte-wise and may split a multi-byte UTF-8 codepoint; the
///   lossy conversion then yields U+FFFD, so a truncated `content.len()` may
///   slightly exceed `max_bytes`. The *buffered bytes* never exceed
///   `max_bytes`.
///
/// # Errors
/// Propagates the underlying reader's I/O errors.
pub async fn read_bounded_line<R>(
    reader: &mut R,
    max_bytes: usize,
) -> std::io::Result<Option<BoundedLine>>
where
    R: AsyncBufRead + Unpin,
{
    // Reserve a modest amount up front to avoid repeated small reallocs on the
    // common short-line path, but never eagerly allocate the full `max_bytes`
    // (which can be several MiB) for what is usually a tiny line.
    let mut buf: Vec<u8> = Vec::with_capacity(max_bytes.min(INITIAL_CAPACITY));
    // `read_bounded_line`'s historical contract excludes the trailing newline
    // from `max_bytes`, while `read_bounded_line_into` caps the raw framed
    // bytes it keeps. Permit one extra byte here so a content-exact line can
    // still keep and strip its newline without being reported as truncated.
    let framed_cap = max_bytes.saturating_add(1);
    let Some(read) = read_bounded_line_into(reader, &mut buf, framed_cap).await? else {
        return Ok(None);
    };
    if buf.ends_with(b"\n") {
        buf.pop();
    }
    if buf.len() > max_bytes {
        buf.truncate(max_bytes);
        buf.shrink_to(max_bytes);
    }
    Ok(Some(BoundedLine {
        content: finish(buf),
        truncated: read.truncated,
    }))
}

/// Read one line from `reader` into a reusable caller-owned byte buffer.
///
/// `line` is cleared before reading. If its capacity is above 64 KiB, the
/// allocation is shrunk toward 8 KiB before reading so a previous oversized
/// frame does not keep memory pinned on the hot path. The function then appends
/// bytes from the next wire line until either `max_bytes` buffered bytes have
/// been kept or the line ends. Any bytes beyond `max_bytes` are still consumed
/// up to the next `\n` (or EOF) but are not appended, preserving stream
/// framing without unbounded buffering.
///
/// Unlike [`read_bounded_line`], this API leaves the bytes exactly as read:
/// the terminating `\n` is included when it fits under `max_bytes`, trailing
/// `\r` is preserved, and invalid UTF-8 remains available for the caller to
/// reject instead of being lossily normalized.
///
/// Returns `Ok(None)` at EOF with no trailing partial data. Returns
/// `Ok(Some(BoundedLineRead { truncated: true, .. }))` when the line on the
/// wire exceeded `max_bytes`; `bytes_read` is the number of bytes consumed from
/// the reader, including discarded bytes and the newline when present. If a
/// previous read grew `line` above the reclaim threshold, the next call shrinks
/// the reusable allocation toward the smaller of the initial reusable capacity
/// and `max_bytes` before reading, so retained memory does not scale with a
/// past legal or oversized message.
///
/// # Caveats
/// - **Not cancellation-safe.** If the returned future is dropped before
///   completion (e.g. losing a `select!` race), bytes already consumed from
///   `reader` for the partially-read line are lost; the next call starts
///   mid-line.
/// - Truncation is byte-wise. Callers converting the kept bytes to UTF-8 decide
///   whether to reject or perform lossy replacement.
///
/// # Errors
/// Propagates the underlying reader's I/O errors.
pub async fn read_bounded_line_into<R>(
    reader: &mut R,
    line: &mut Vec<u8>,
    max_bytes: usize,
) -> std::io::Result<Option<BoundedLineRead>>
where
    R: AsyncBufRead + Unpin,
{
    line.clear();
    // Reclaim memory before reading so the shrink happens even when the caller
    // will later skip its loop tail for oversized, invalid, or empty frames.
    if line.capacity() > RECLAIM_THRESHOLD {
        line.shrink_to(INITIAL_CAPACITY.min(max_bytes));
    }

    let mut bytes_read = 0;
    let mut truncated = false;
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok((bytes_read > 0 || truncated).then_some(BoundedLineRead {
                bytes_read,
                truncated,
            }));
        }

        let newline_pos = available.iter().position(|&byte| byte == b'\n');
        let take = newline_pos.map_or(available.len(), |pos| pos + 1);
        let copy_len = max_bytes.saturating_sub(line.len()).min(take);
        if copy_len < take {
            truncated = true;
        }

        if line.len() + copy_len > line.capacity() {
            let target = (line.capacity() * 2)
                .max(line.len() + copy_len)
                .min(max_bytes);
            line.reserve_exact(target - line.len());
        }
        line.extend_from_slice(&available[..copy_len]);
        bytes_read += take;
        reader.consume(take);

        if newline_pos.is_some() {
            return Ok(Some(BoundedLineRead {
                bytes_read,
                truncated,
            }));
        }
    }
}

mod frame;
pub use frame::{
    FrameError, Incoming, PROTOCOL_ANNOUNCE_PREFIX, PROTOCOL_FRAME_CAP_BYTES, PROTOCOL_VERSION,
    WireMode, read_frame, read_frame_or_legacy_line, read_frame_or_legacy_line_into, write_frame,
    write_message,
};

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
