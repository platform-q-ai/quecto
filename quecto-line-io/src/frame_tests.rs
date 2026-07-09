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
    // then read it back under a small cap (ADR-0010 build_oversized_frame
    // pattern: malformed input constructed against the real format).
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
