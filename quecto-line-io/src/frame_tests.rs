//! Unit tests for the ADR-0008 part 1 frame writer/reader (#1059).

use super::*;
use tokio::io::BufReader;

async fn framed(payloads: &[&[u8]]) -> Vec<u8> {
    let mut wire = Vec::new();
    for p in payloads {
        write_frame(&mut wire, p, PROTOCOL_FRAME_CAP_BYTES)
            .await
            .expect("test frame under cap must be writable");
    }
    wire
}

/// AC: payloads stay UTF-8 JSON, unchanged in shape — a write/read
/// round-trip returns the exact payload bytes.
#[tokio::test]
async fn frame_roundtrip_preserves_json_payload_bytes() {
    let payload = br#"{"type":"prompt","text":"hi"}"#;
    let wire = framed(&[payload]).await;
    let mut r = BufReader::new(&wire[..]);
    let got = read_frame(&mut r, PROTOCOL_FRAME_CAP_BYTES)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got, payload);
}

/// AC: frames are delimited by their declared size — multiple frames on
/// one stream read back in order, then clean EOF yields `None`.
#[tokio::test]
async fn multiple_frames_read_in_sequence_then_eof() {
    let wire = framed(&[br#"{"n":1}"#, br#"{"n":2}"#]).await;
    let mut r = BufReader::new(&wire[..]);
    assert_eq!(
        read_frame(&mut r, PROTOCOL_FRAME_CAP_BYTES)
            .await
            .unwrap()
            .unwrap(),
        br#"{"n":1}"#
    );
    assert_eq!(
        read_frame(&mut r, PROTOCOL_FRAME_CAP_BYTES)
            .await
            .unwrap()
            .unwrap(),
        br#"{"n":2}"#
    );
    assert!(
        read_frame(&mut r, PROTOCOL_FRAME_CAP_BYTES)
            .await
            .unwrap()
            .is_none()
    );
}

/// AC: the reader learns the size before the payload is read; an
/// over-limit frame is rejected with a clean protocol error naming the
/// declared size and the cap (no mid-buffer truncation, no silent drop).
#[tokio::test]
async fn over_limit_frame_is_rejected_with_declared_size() {
    // Author the oversized frame against the real writer at a large cap,
    // then read it back under a small cap (ADR-0011 build_oversized_frame
    // pattern, retained from superseded ADR-0010: malformed input constructed
    // against the real format).
    let big = vec![b'x'; 64];
    let mut wire = Vec::new();
    write_frame(&mut wire, &big, 1024).await.unwrap();
    let mut r = BufReader::new(&wire[..]);
    match read_frame(&mut r, 16).await {
        Err(FrameError::Oversized { declared, max }) => {
            assert_eq!(declared, 64);
            assert_eq!(max, 16);
        }
        other => panic!("expected Oversized error, got {other:?}"),
    }
}

/// AC: after an over-limit frame is rejected, the connection's
/// subsequent frames are still processed (the declared payload is
/// consumed, keeping the stream framed).
#[tokio::test]
async fn frames_after_a_rejected_over_limit_frame_are_still_processed() {
    let big = vec![b'x'; 64];
    let mut wire = Vec::new();
    write_frame(&mut wire, &big, 1024).await.unwrap();
    write_frame(&mut wire, br#"{"ok":true}"#, 1024)
        .await
        .unwrap();
    let mut r = BufReader::new(&wire[..]);
    assert!(matches!(
        read_frame(&mut r, 16).await,
        Err(FrameError::Oversized { .. })
    ));
    let next = read_frame(&mut r, 1024).await.unwrap().unwrap();
    assert_eq!(next, br#"{"ok":true}"#);
}

/// Boundary (coverage review): a payload of exactly `max_bytes` is legal
/// on BOTH sides — the writer emits it and the reader returns it — so
/// GREEN cannot silently implement the cap as `>=`.
#[tokio::test]
async fn payload_exactly_at_cap_round_trips() {
    let payload = vec![b'x'; 16];
    let mut wire = Vec::new();
    write_frame(&mut wire, &payload, 16)
        .await
        .expect("a payload of exactly max_bytes must be writable");
    let mut r = BufReader::new(&wire[..]);
    let got = read_frame(&mut r, 16)
        .await
        .expect("a frame of exactly max_bytes must be readable")
        .unwrap();
    assert_eq!(got, payload);
}

/// Boundary (coverage review): one byte over the cap is rejected by both
/// the writer and the reader — pinning the strictly-greater semantics.
#[tokio::test]
async fn payload_one_byte_over_cap_is_rejected_on_both_sides() {
    let payload = vec![b'x'; 17];
    let mut wire = Vec::new();
    assert!(matches!(
        write_frame(&mut wire, &payload, 16).await,
        Err(FrameError::Oversized {
            declared: 17,
            max: 16
        })
    ));
    // Author against the real writer at a larger cap, then read under 16.
    write_frame(&mut wire, &payload, 17).await.unwrap();
    let mut r = BufReader::new(&wire[..]);
    assert!(matches!(
        read_frame(&mut r, 16).await,
        Err(FrameError::Oversized {
            declared: 17,
            max: 16
        })
    ));
}

/// AC (falsifiability review): the declared size is learned from the
/// prefix alone — an over-cap declaration whose payload the peer withholds
/// (stream ends after the prefix) is still rejected as `Oversized`, which
/// a buffer-the-payload-first implementation could not produce.
#[tokio::test]
async fn over_limit_declaration_is_rejected_from_the_prefix_alone() {
    // Author the oversized frame against the real writer, then keep only
    // its length prefix — the payload bytes never arrive.
    let mut wire = Vec::new();
    write_frame(&mut wire, &[b'x'; 64], 1024).await.unwrap();
    wire.truncate(wire.len() - 64);
    let mut r = BufReader::new(&wire[..]);
    match read_frame(&mut r, 16).await {
        Err(FrameError::Oversized { declared, max }) => {
            assert_eq!(declared, 64);
            assert_eq!(max, 16);
        }
        other => panic!("expected Oversized from the prefix alone, got {other:?}"),
    }
}

/// AC (emitter symmetry): an emitter refuses to produce a frame above
/// the cap instead of writing something a reader must reject.
#[tokio::test]
async fn write_frame_refuses_over_limit_payloads() {
    let mut wire = Vec::new();
    let big = vec![b'x'; 32];
    match write_frame(&mut wire, &big, 16).await {
        Err(FrameError::Oversized { declared, max }) => {
            assert_eq!(declared, 32);
            assert_eq!(max, 16);
        }
        other => panic!("expected Oversized error, got {other:?}"),
    }
    assert!(wire.is_empty(), "no partial frame may reach the wire");
}

/// Compatibility emit boundary: the legacy cap includes the trailing newline,
/// so payload + newline exactly at `max_bytes` is legal.
#[tokio::test]
async fn write_message_legacy_line_exactly_at_wire_cap_succeeds() {
    let mut wire = Vec::new();
    write_message(&mut wire, b"1234567", WireMode::LegacyLine, 8)
        .await
        .expect("an eight-byte legacy wire line must fit an eight-byte cap");
    assert_eq!(wire, b"1234567\n");
}

/// Compatibility emit boundary: exceeding the whole legacy wire-line cap by
/// one byte is rejected before either payload or newline reaches the writer.
#[tokio::test]
async fn write_message_legacy_line_one_byte_over_wire_cap_writes_nothing() {
    let mut wire = Vec::new();
    assert!(matches!(
        write_message(&mut wire, b"12345678", WireMode::LegacyLine, 8).await,
        Err(FrameError::Oversized {
            declared: 9,
            max: 8
        })
    ));
    assert!(wire.is_empty(), "no partial legacy line may reach the wire");
}

/// `write_message` retains framed payload-cap semantics: the prefix is not
/// counted against `max_bytes` and an exactly-at-cap payload is emitted.
#[tokio::test]
async fn write_message_framed_mode_retains_payload_cap_semantics() {
    let mut wire = Vec::new();
    write_message(&mut wire, b"12345678", WireMode::Framed, 8)
        .await
        .expect("an eight-byte framed payload must fit an eight-byte cap");
    assert_eq!(&wire[..4], &8u32.to_be_bytes());
    assert_eq!(&wire[4..], b"12345678");
}

/// AC (deprecation window): a legacy NDJSON peer's `{`-opening line is
/// detected and read as a legacy line, not misparsed as a frame.
#[tokio::test]
async fn sniff_detects_legacy_ndjson_peer_and_reads_its_line() {
    let wire = b"{\"type\":\"prompt\"}\n{\"type\":\"close\"}\n";
    let mut r = BufReader::new(&wire[..]);
    let first = read_frame_or_legacy_line(&mut r, PROTOCOL_FRAME_CAP_BYTES)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        first,
        Incoming::LegacyLine(br#"{"type":"prompt"}"#.to_vec())
    );
    let second = read_frame_or_legacy_line(&mut r, PROTOCOL_FRAME_CAP_BYTES)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        second,
        Incoming::LegacyLine(br#"{"type":"close"}"#.to_vec())
    );
}

/// AC (deprecation window): a new-framing peer is detected from its
/// first byte and its frames are read as frames.
#[tokio::test]
async fn sniff_detects_framed_peer_and_reads_its_frames() {
    let wire = framed(&[br#"{"type":"prompt"}"#]).await;
    let mut r = BufReader::new(&wire[..]);
    let got = read_frame_or_legacy_line(&mut r, PROTOCOL_FRAME_CAP_BYTES)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got, Incoming::Frame(br#"{"type":"prompt"}"#.to_vec()));
}

/// AC: a peer speaking neither framing fails with an explicit,
/// diagnosable version-mismatch error — never silent misparsing or a
/// hang. 0xFF is neither `{` nor a valid frame prefix.
#[tokio::test]
async fn unknown_first_byte_is_an_explicit_version_mismatch() {
    let wire = [0xFFu8, 0x00, 0x01, 0x02];
    let mut r = BufReader::new(&wire[..]);
    match read_frame_or_legacy_line(&mut r, PROTOCOL_FRAME_CAP_BYTES).await {
        Err(FrameError::VersionMismatch { first_byte }) => assert_eq!(first_byte, 0xFF),
        other => panic!("expected VersionMismatch error, got {other:?}"),
    }
}

/// AC: the version-mismatch error is diagnosable — its message names the
/// offending byte and the mismatch so a human can act on it.
#[test]
fn version_mismatch_error_message_is_diagnosable() {
    let msg = FrameError::VersionMismatch { first_byte: 0xFF }.to_string();
    assert!(msg.contains("version mismatch"), "message was: {msg}");
    assert!(msg.contains("0xff"), "message was: {msg}");
}

/// AC: the oversized-frame error is a clean, loggable protocol error
/// naming the declared size and the cap.
#[test]
fn oversized_error_message_is_diagnosable() {
    let msg = FrameError::Oversized {
        declared: 64,
        max: 16,
    }
    .to_string();
    assert!(msg.contains("64"), "message was: {msg}");
    assert!(msg.contains("16"), "message was: {msg}");
}

/// The buffer-reusing reader returns the same payload bytes as the allocating
/// twin for both framings, and reuses the caller's `Vec` allocation across
/// calls (no per-message alloc on the hot event path, #1059 review).
#[tokio::test]
async fn into_reader_reuses_buffer_across_framed_and_legacy_messages() {
    // A framed message, then a legacy NDJSON line, on one stream.
    let mut wire = framed(&[br#"{"n":1}"#]).await;
    wire.extend_from_slice(b"{\"n\":2}\n");
    let mut r = BufReader::new(&wire[..]);

    let mut buf = Vec::with_capacity(8 * 1024);
    let ptr = buf.as_ptr();

    let mode = read_frame_or_legacy_line_into(&mut r, &mut buf, PROTOCOL_FRAME_CAP_BYTES)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(mode, WireMode::Framed);
    assert_eq!(buf, br#"{"n":1}"#);
    assert_eq!(
        buf.as_ptr(),
        ptr,
        "framed read must reuse the caller buffer"
    );

    let mode = read_frame_or_legacy_line_into(&mut r, &mut buf, PROTOCOL_FRAME_CAP_BYTES)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(mode, WireMode::LegacyLine);
    assert_eq!(
        buf, br#"{"n":2}"#,
        "legacy line must have its newline stripped"
    );
    assert_eq!(
        buf.as_ptr(),
        ptr,
        "legacy read must reuse the caller buffer"
    );

    assert!(
        read_frame_or_legacy_line_into(&mut r, &mut buf, PROTOCOL_FRAME_CAP_BYTES)
            .await
            .unwrap()
            .is_none(),
        "clean EOF yields None"
    );
}

/// The buffer-reusing reader rejects an over-cap frame cleanly (declared size
/// learned from the prefix, payload discarded) and keeps the stream framed so a
/// following in-cap frame still reads.
#[tokio::test]
async fn into_reader_rejects_oversized_frame_then_reads_the_next() {
    // Hand-build an over-cap frame prefix (declares 32 bytes) with a small cap
    // of 8, followed by a legal in-cap frame.
    let mut wire = Vec::new();
    wire.extend_from_slice(&32u32.to_be_bytes());
    wire.extend_from_slice(&[b'x'; 32]);
    write_frame(&mut wire, br#"{"n":9}"#, 8).await.unwrap();
    let mut r = BufReader::new(&wire[..]);

    let mut buf = Vec::new();
    match read_frame_or_legacy_line_into(&mut r, &mut buf, 8).await {
        Err(FrameError::Oversized { declared, max }) => {
            assert_eq!(declared, 32);
            assert_eq!(max, 8);
        }
        other => panic!("expected Oversized, got {other:?}"),
    }
    let mode = read_frame_or_legacy_line_into(&mut r, &mut buf, 8)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(mode, WireMode::Framed);
    assert_eq!(buf, br#"{"n":9}"#);
}

/// The buffer-reusing reader surfaces an unknown first byte as an explicit
/// version mismatch (never a silent misparse), matching the allocating twin.
#[tokio::test]
async fn into_reader_reports_version_mismatch_on_unknown_first_byte() {
    let wire = [0xFFu8, 0x00, 0x01, 0x02];
    let mut r = BufReader::new(&wire[..]);
    let mut buf = Vec::new();
    match read_frame_or_legacy_line_into(&mut r, &mut buf, PROTOCOL_FRAME_CAP_BYTES).await {
        Err(FrameError::VersionMismatch { first_byte }) => assert_eq!(first_byte, 0xFF),
        other => panic!("expected VersionMismatch, got {other:?}"),
    }
}

fn assert_unexpected_eof(err: FrameError) {
    match err {
        FrameError::Io(e) => assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof),
        other => panic!("expected UnexpectedEof, got {other:?}"),
    }
}

#[tokio::test]
async fn eof_matrix_matches_contract_for_allocating_and_into_framed_readers() {
    for prefix_len in 1..FRAME_PREFIX_LEN {
        let wire = vec![0u8; prefix_len];
        let mut r = BufReader::new(&wire[..]);
        assert_unexpected_eof(read_frame(&mut r, 8).await.unwrap_err());

        let mut r = BufReader::new(&wire[..]);
        let mut buf = Vec::new();
        assert_unexpected_eof(
            read_frame_or_legacy_line_into(&mut r, &mut buf, 8)
                .await
                .unwrap_err(),
        );
        assert!(
            buf.is_empty(),
            "partial prefix must not publish payload bytes"
        );
    }

    let partial_payload = [0, 0, 0, 8, b'a', b'b', b'c'];
    let mut r = BufReader::new(&partial_payload[..]);
    assert_unexpected_eof(read_frame(&mut r, 8).await.unwrap_err());

    let mut r = BufReader::new(&partial_payload[..]);
    let mut buf = Vec::new();
    assert_unexpected_eof(
        read_frame_or_legacy_line_into(&mut r, &mut buf, 8)
            .await
            .unwrap_err(),
    );
    assert_ne!(
        buf, b"abc",
        "partial payload must not be accepted as a message"
    );

    let over_cap_prefix_only = 9u32.to_be_bytes();
    let mut r = BufReader::new(&over_cap_prefix_only[..]);
    assert!(matches!(
        read_frame(&mut r, 8).await,
        Err(FrameError::Oversized {
            declared: 9,
            max: 8
        })
    ));

    let mut r = BufReader::new(&over_cap_prefix_only[..]);
    let mut buf = Vec::new();
    assert!(matches!(
        read_frame_or_legacy_line_into(&mut r, &mut buf, 8).await,
        Err(FrameError::Oversized {
            declared: 9,
            max: 8
        })
    ));
}

#[tokio::test]
async fn oversized_legacy_line_recovers_only_after_delimiter_and_eof_is_clean() {
    let mut wire = Vec::new();
    wire.extend_from_slice(b"{");
    wire.extend_from_slice(&[b'x'; 40]);
    wire.extend_from_slice(b"\n{\"ok\":true}\n");
    let mut r = BufReader::new(&wire[..]);
    let mut buf = Vec::new();
    assert!(matches!(
        read_frame_or_legacy_line_into(&mut r, &mut buf, 32).await,
        Err(FrameError::Oversized { .. })
    ));
    let mode = read_frame_or_legacy_line_into(&mut r, &mut buf, 32)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(mode, WireMode::LegacyLine);
    assert_eq!(buf, br#"{"ok":true}"#);

    let mut unterminated = Vec::new();
    unterminated.extend_from_slice(b"{");
    unterminated.extend_from_slice(&[b'x'; 16]);
    let mut r = BufReader::new(&unterminated[..]);
    let mut buf = Vec::new();
    assert!(matches!(
        read_frame_or_legacy_line_into(&mut r, &mut buf, 8).await,
        Err(FrameError::Oversized { .. })
    ));
    assert!(
        read_frame_or_legacy_line_into(&mut r, &mut buf, 8)
            .await
            .unwrap()
            .is_none(),
        "legacy EOF without a delimiter cannot recover another message"
    );
}

#[tokio::test]
async fn framed_reader_preserves_invalid_utf8_payload_bytes() {
    let payload = [b'a', 0xFF, b'b'];
    let wire = framed(&[&payload]).await;
    let mut r = BufReader::new(&wire[..]);
    let got = read_frame_or_legacy_line(&mut r, PROTOCOL_FRAME_CAP_BYTES)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got, Incoming::Frame(payload.to_vec()));
}

#[tokio::test]
async fn reusable_frame_reader_reclaims_after_large_and_oversized_messages() {
    let large = vec![b'x'; 70_000];
    let small = b"{}";
    let mut wire = framed(&[&large, small]).await;
    let mut r = BufReader::new(&wire[..]);
    let mut buf = Vec::new();
    assert_eq!(
        read_frame_or_legacy_line_into(&mut r, &mut buf, PROTOCOL_FRAME_CAP_BYTES)
            .await
            .unwrap(),
        Some(WireMode::Framed)
    );
    assert!(
        buf.capacity() > 64 * 1024,
        "test setup must grow the buffer"
    );
    assert_eq!(
        read_frame_or_legacy_line_into(&mut r, &mut buf, PROTOCOL_FRAME_CAP_BYTES)
            .await
            .unwrap(),
        Some(WireMode::Framed)
    );
    assert_eq!(buf, small);
    assert!(buf.capacity() <= 8 * 1024);

    wire.clear();
    wire.extend_from_slice(&70_000u32.to_be_bytes());
    wire.extend_from_slice(&vec![b'x'; 70_000]);
    write_frame(&mut wire, small, PROTOCOL_FRAME_CAP_BYTES)
        .await
        .unwrap();
    let mut r = BufReader::new(&wire[..]);
    buf = Vec::with_capacity(128 * 1024);
    assert!(matches!(
        read_frame_or_legacy_line_into(&mut r, &mut buf, 8).await,
        Err(FrameError::Oversized { .. })
    ));
    assert_eq!(
        read_frame_or_legacy_line_into(&mut r, &mut buf, PROTOCOL_FRAME_CAP_BYTES)
            .await
            .unwrap(),
        Some(WireMode::Framed)
    );
    assert_eq!(buf, small);
    assert!(buf.capacity() <= 8 * 1024);
}
