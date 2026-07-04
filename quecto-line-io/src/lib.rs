//! Shared bounded line reader for quecto's JSON-lines UDS protocol.
//!
//! Several crates (`quecto-api`, `quecto-agentic-harness`) read
//! `\n`-terminated JSON lines off a `UnixStream`/pipe from a peer that must be
//! treated as untrusted (a UDS client, a spawned sub-agent, or a parent
//! process). Reading with `AsyncBufReadExt::lines()`/`read_line` buffers the
//! *entire* line before any length check can run, so one giant unterminated
//! line can grow the buffer without bound before it's dropped.
//!
//! `quecto-tui` implements the same guarantee separately: its reader reuses a
//! caller-owned buffer (with `shrink_to` reclaim) to avoid per-line allocation
//! on the hot render path, which this allocate-per-call API does not yet offer.
//! Migrating it here is deferred until this crate grows a
//! `read_bounded_line_into(&mut Vec<u8>, ..)` variant.
//!
//! [`read_bounded_line`] instead caps memory growth *while* reading: once the
//! accumulated line reaches `max_bytes`, any further bytes up to the next
//! `\n` are read from the socket (so the stream stays framed) but discarded
//! rather than appended, so the buffer never exceeds `max_bytes`. Callers
//! that want the exceeded case to be a hard read error (e.g. a required
//! response) can check [`BoundedLine::truncated`] and translate it into their
//! own error type.

use tokio::io::{AsyncBufRead, AsyncBufReadExt};

/// A single `\n`-terminated (or EOF-terminated final) line, capped to at most
/// `max_bytes` of content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedLine {
    /// The line's content, excluding the trailing `\n`, capped to `max_bytes`.
    pub content: String,
    /// `true` if the source line on the wire was longer than `max_bytes` and
    /// bytes beyond the cap were discarded (rather than buffered).
    pub truncated: bool,
}

const INITIAL_CAPACITY: usize = 8 * 1024;

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
    let mut truncated = false;
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            // EOF: surface any trailing partial line, else signal closed stream.
            return Ok((!buf.is_empty() || truncated).then(|| BoundedLine {
                content: finish(buf),
                truncated,
            }));
        }
        let newline_pos = available.iter().position(|&b| b == b'\n');
        let take = newline_pos.map_or(available.len(), |pos| pos + 1);
        // Only copy up to the amount of room remaining under the cap; any
        // bytes beyond that within this chunk are consumed (so the stream
        // stays framed) but never appended to `buf`.
        let usable = newline_pos.unwrap_or(available.len());
        let copy_len = max_bytes.saturating_sub(buf.len()).min(usable);
        if copy_len < usable {
            truncated = true;
        }
        buf.extend_from_slice(&available[..copy_len]);
        reader.consume(take);
        if newline_pos.is_some() {
            return Ok(Some(BoundedLine {
                content: finish(buf),
                truncated,
            }));
        }
    }
}

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
}
