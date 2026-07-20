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
