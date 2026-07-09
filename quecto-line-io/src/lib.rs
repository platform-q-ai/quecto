//! Shared bounded line reader for quecto's JSON-lines UDS protocol.
//!
//! Several crates (`quecto-api`, `quecto-agentic-harness`) read
//! `\n`-terminated JSON lines off a `UnixStream`/pipe from a peer that must be
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

/// Single source of truth for quecto's JSON-lines protocol per-line cap
/// (1 MiB, INCLUDING the trailing `\n`).
///
/// Every reader bound and emitter cap in the workspace derives from this
/// constant (`quecto-tui`'s `MAX_LINE_BYTES`, the harness's
/// `EVENT_LINE_CAP_BYTES` and UDS/sub-agent read bounds, `quecto-api`'s
/// client bound), so an emitter can never legally produce a line a reader
/// drops unread (#1047). Hand-pinning the value in a dependent crate instead
/// of referencing this constant reintroduces that failure mode.
pub const PROTOCOL_LINE_CAP_BYTES: usize = 1_048_576;

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
    // (which can be 1 MiB) for what is usually a tiny line.
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
/// the reader, including discarded bytes and the newline when present.
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
mod tests {
    use super::*;
    use tokio::io::BufReader;

    fn reader(bytes: &'static [u8]) -> BufReader<&'static [u8]> {
        BufReader::new(bytes)
    }

    #[tokio::test]
    async fn reads_a_normal_line() {
        let mut r = reader(b"hello\n");
        let line = read_bounded_line(&mut r, 1024).await.unwrap().unwrap();
        assert_eq!(line.content, "hello");
        assert!(!line.truncated);
    }

    #[tokio::test]
    async fn reads_multiple_lines_in_sequence() {
        let mut r = reader(b"one\ntwo\n");
        let a = read_bounded_line(&mut r, 1024).await.unwrap().unwrap();
        let b = read_bounded_line(&mut r, 1024).await.unwrap().unwrap();
        assert_eq!(a.content, "one");
        assert_eq!(b.content, "two");
    }

    #[tokio::test]
    async fn returns_none_at_clean_eof() {
        let mut r = reader(b"");
        assert!(read_bounded_line(&mut r, 1024).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn surfaces_trailing_partial_line_at_eof() {
        let mut r = reader(b"no newline");
        let line = read_bounded_line(&mut r, 1024).await.unwrap().unwrap();
        assert_eq!(line.content, "no newline");
        assert!(!line.truncated);
    }

    /// Boundary: a line exactly at the cap is NOT truncated.
    #[tokio::test]
    async fn line_exactly_at_cap_is_not_truncated() {
        let payload = "a".repeat(10);
        let mut input = payload.clone().into_bytes();
        input.push(b'\n');
        let mut r = BufReader::new(&input[..]);
        let line = read_bounded_line(&mut r, 10).await.unwrap().unwrap();
        assert_eq!(line.content, payload);
        assert!(!line.truncated);
    }

    /// Boundary: a line one byte over the cap IS truncated, and content is
    /// capped to exactly `max_bytes`.
    #[tokio::test]
    async fn line_one_byte_over_cap_is_truncated() {
        let payload = "a".repeat(11);
        let mut input = payload.into_bytes();
        input.push(b'\n');
        let mut r = BufReader::new(&input[..]);
        let line = read_bounded_line(&mut r, 10).await.unwrap().unwrap();
        assert_eq!(line.content.len(), 10);
        assert!(line.truncated);
    }

    /// A giant unterminated line must not grow the buffer past `max_bytes`,
    /// even though many chunks are consumed before the terminator arrives.
    #[tokio::test]
    async fn oversized_line_never_buffers_past_cap() {
        // Simulate a line built of many small fill_buf() chunks by using a
        // reader with tiny internal capacity over a large payload.
        let mut payload = vec![b'x'; 5000];
        payload.push(b'\n');
        let r = tokio::io::BufReader::with_capacity(16, &payload[..]);
        let mut r = r;
        let line = read_bounded_line(&mut r, 100).await.unwrap().unwrap();
        assert_eq!(line.content.len(), 100);
        assert!(line.truncated);
        // The valid-UTF-8 path in `finish` reuses the accumulation buffer's
        // allocation verbatim, so this observes the internal buffer's real
        // capacity: growth is capped at `max_bytes`, and a regression to
        // "buffer everything, truncate post-hoc" (or unchecked doubling past
        // the cap) fails here.
        assert!(
            line.content.capacity() <= 100,
            "buffer capacity {} exceeded max_bytes",
            line.content.capacity()
        );
    }

    /// After an oversized line, the framing must still be intact so the next
    /// line reads correctly (the discarded tail must still be consumed).
    #[tokio::test]
    async fn framing_preserved_after_oversized_line() {
        let mut input = vec![b'x'; 50];
        input.push(b'\n');
        input.extend_from_slice(b"next\n");
        let mut r = BufReader::new(&input[..]);
        let first = read_bounded_line(&mut r, 10).await.unwrap().unwrap();
        assert!(first.truncated);
        let second = read_bounded_line(&mut r, 10).await.unwrap().unwrap();
        assert_eq!(second.content, "next");
        assert!(!second.truncated);
    }

    /// Invalid UTF-8 must not panic or error — `finish` falls back to a lossy
    /// conversion (U+FFFD replacement) rather than reusing the buffer. This
    /// pins the fallback arm added alongside the zero-copy `String::from_utf8`
    /// fast path; reverting `finish` to `from_utf8().unwrap()` breaks this.
    #[tokio::test]
    async fn invalid_utf8_falls_back_to_lossy() {
        // 0xFF is never valid UTF-8.
        let input = [b'a', 0xFF, b'b', b'\n'];
        let mut r = BufReader::new(&input[..]);
        let line = read_bounded_line(&mut r, 1024).await.unwrap().unwrap();
        assert_eq!(line.content, "a\u{FFFD}b");
        assert!(!line.truncated);
    }

    /// An oversized line that ends at EOF *without* a terminating `\n` must
    /// still be surfaced (with `truncated: true`) via the `|| truncated` arm
    /// of the EOF branch, and the following call must report clean EOF.
    #[tokio::test]
    async fn oversized_unterminated_line_at_eof_is_surfaced_as_truncated() {
        let payload = vec![b'x'; 500]; // no trailing '\n'
        let mut r = tokio::io::BufReader::with_capacity(16, &payload[..]);
        let line = read_bounded_line(&mut r, 100).await.unwrap().unwrap();
        assert!(line.truncated);
        assert_eq!(line.content.len(), 100);
        assert!(
            read_bounded_line(&mut r, 100).await.unwrap().is_none(),
            "the call after the truncated EOF line must return None"
        );
    }

    /// Pins that a trailing `\r` is preserved — unlike
    /// `AsyncBufReadExt::lines()`, which strips `\r\n`. Documented in the
    /// `read_bounded_line` caveats; callers must trim it themselves.
    #[tokio::test]
    async fn trailing_carriage_return_is_preserved() {
        let mut r = reader(b"hello\r\n");
        let line = read_bounded_line(&mut r, 1024).await.unwrap().unwrap();
        assert_eq!(line.content, "hello\r");
        assert!(!line.truncated);
    }

    /// Byte-wise truncation may split a multi-byte UTF-8 codepoint; the lossy
    /// conversion must yield U+FFFD for the dangling prefix without panicking.
    /// Note `content.len()` may exceed `max_bytes` on this path (U+FFFD is 3
    /// bytes), which the docs call out.
    #[tokio::test]
    async fn truncation_mid_codepoint_yields_replacement_character() {
        // "é" is 2 bytes (0xC3 0xA9); cap at 4 bytes so truncation lands
        // after the first byte of the second "é".
        let mut input = "aaaéé".as_bytes().to_vec(); // 3 + 2 + 2 = 7 bytes
        input.push(b'\n');
        let mut r = BufReader::new(&input[..]);
        let line = read_bounded_line(&mut r, 4).await.unwrap().unwrap();
        assert!(line.truncated);
        assert_eq!(line.content, "aaa\u{FFFD}");
    }

    #[tokio::test]
    async fn into_reuses_caller_buffer_and_preserves_framed_bytes() {
        let mut r = reader(b"one\ntwo\n");
        let mut buf = Vec::with_capacity(128);
        let original_ptr = buf.as_ptr();

        let first = read_bounded_line_into(&mut r, &mut buf, 1024)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.bytes_read, 4);
        assert!(!first.truncated);
        assert_eq!(buf, b"one\n");
        assert_eq!(buf.as_ptr(), original_ptr);

        let second = read_bounded_line_into(&mut r, &mut buf, 1024)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second.bytes_read, 4);
        assert!(!second.truncated);
        assert_eq!(buf, b"two\n");
        assert_eq!(buf.as_ptr(), original_ptr);
    }

    #[tokio::test]
    async fn into_reclaims_large_capacity_before_reading_next_line() {
        let mut input = vec![b'x'; 100_000];
        input.push(b'\n');
        input.extend_from_slice(b"ok\n");
        let mut r = tokio::io::BufReader::with_capacity(16, &input[..]);
        let mut buf = Vec::new();

        let oversized = read_bounded_line_into(&mut r, &mut buf, 70_000)
            .await
            .unwrap()
            .unwrap();
        assert!(oversized.truncated);
        assert_eq!(buf.len(), 70_000);
        assert!(
            buf.capacity() > 64 * 1024,
            "test setup must force the reusable buffer above the reclaim threshold; capacity was {}",
            buf.capacity()
        );

        let next = read_bounded_line_into(&mut r, &mut buf, 70_000)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(next.bytes_read, 3);
        assert!(!next.truncated);
        assert_eq!(buf, b"ok\n");
        assert!(
            buf.capacity() <= 8 * 1024,
            "the reusable buffer should be reclaimed before the next read; capacity was {}",
            buf.capacity()
        );
    }

    #[tokio::test]
    async fn into_reclaims_only_above_threshold() {
        let mut r = reader(b"ok\n");
        let mut buf = Vec::with_capacity(64 * 1024);
        let original_capacity = buf.capacity();

        let read = read_bounded_line_into(&mut r, &mut buf, 70_000)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(read.bytes_read, 3);
        assert_eq!(buf, b"ok\n");
        assert_eq!(buf.capacity(), original_capacity);
    }

    #[tokio::test]
    async fn into_exact_cap_and_one_byte_over_boundaries() {
        let mut input = b"abc\n".to_vec();
        input.extend_from_slice(b"abcd\n");
        let mut r = BufReader::new(&input[..]);
        let mut buf = Vec::new();

        let exact = read_bounded_line_into(&mut r, &mut buf, 4)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(exact.bytes_read, 4);
        assert!(!exact.truncated);
        assert_eq!(buf, b"abc\n");

        let over = read_bounded_line_into(&mut r, &mut buf, 4)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(over.bytes_read, 5);
        assert!(over.truncated);
        assert_eq!(buf, b"abcd");
    }

    #[tokio::test]
    async fn into_preserves_invalid_utf8_bytes_for_callers_to_reject() {
        let input = [b'a', 0xFF, b'b', b'\n'];
        let mut r = BufReader::new(&input[..]);
        let mut buf = Vec::new();

        let read = read_bounded_line_into(&mut r, &mut buf, 1024)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(read.bytes_read, 4);
        assert!(!read.truncated);
        assert_eq!(buf, input);
        assert!(std::str::from_utf8(&buf).is_err());
    }

    #[tokio::test]
    async fn into_discards_oversized_tail_and_resumes_at_next_line() {
        let mut input = vec![b'x'; 50];
        input.push(b'\n');
        input.extend_from_slice(b"next\n");
        let mut r = BufReader::new(&input[..]);
        let mut buf = Vec::new();

        let first = read_bounded_line_into(&mut r, &mut buf, 10)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.bytes_read, 51);
        assert!(first.truncated);
        assert_eq!(buf.len(), 10);
        assert!(!buf.ends_with(b"\n"));

        let second = read_bounded_line_into(&mut r, &mut buf, 10)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second.bytes_read, 5);
        assert!(!second.truncated);
        assert_eq!(buf, b"next\n");
    }

    #[tokio::test]
    async fn into_returns_none_at_clean_eof() {
        let mut r = reader(b"");
        let mut buf = b"stale".to_vec();
        assert!(
            read_bounded_line_into(&mut r, &mut buf, 1024)
                .await
                .unwrap()
                .is_none()
        );
        assert!(buf.is_empty());
    }
}
