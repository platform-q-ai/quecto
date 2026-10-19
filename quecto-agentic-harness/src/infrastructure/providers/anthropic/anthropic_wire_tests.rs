//! The Messages (Anthropic) dialect (#2597): the output each event
//! carries, how `event:` lines name events, and its error wording — moved
//! out of `transport/` with the vendor-neutral contract.
use super::*;
use crate::infrastructure::providers::transport::attempt_profile::Surface;
use serde_json::json;

#[test]
fn an_anthropic_delta_counts_text_thinking_and_tool_input() {
    let cases = [
        (
            json!({"index": 0, "delta": {"type": "text_delta", "text": "hello"}}),
            5,
        ),
        (
            json!({"index": 0, "delta": {"type": "thinking_delta", "thinking": "hmm"}}),
            3,
        ),
        (
            json!({"index": 1, "delta": {"type": "input_json_delta", "partial_json": "{\"a\""}}),
            4,
        ),
        (
            json!({"index": 0, "delta": {"type": "signature_delta", "signature": "sig"}}),
            0,
        ),
        (
            json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}}),
            0,
        ),
        (json!({"type": "ping"}), 0),
    ];
    for (event, bytes) in cases {
        assert_eq!(ANTHROPIC_WIRE.output_bytes(&event), bytes, "{event}");
    }
}

/// An `event:` line names the data after it: by a name the harness knows,
/// or by none (#2433), so a record holds no name a provider made up.
#[test]
fn an_event_line_names_a_known_event_or_none() {
    for known in [
        "error",
        "message_stop",
        "message_start",
        "message_delta",
        "content_block_start",
        "content_block_delta",
        "content_block_stop",
        "ping",
    ] {
        assert_eq!(ANTHROPIC_WIRE.event_line(known), Some(known));
        assert_eq!(ANTHROPIC_WIRE.event_kind(known, &json!({})), known);
    }
    assert_eq!(ANTHROPIC_WIRE.event_line("brand_new"), Some(""));
    assert_eq!(ANTHROPIC_WIRE.event_kind("", &json!({})), "(unknown)");
}

/// The Anthropic wording: an assembled stream's failed read names the
/// stream, and only the chat surface is strict about its error body.
#[test]
fn anthropic_reads_its_stream_and_is_strict_only_on_chat() {
    assert_eq!(ANTHROPIC_WIRE.name(), "Anthropic");
    assert_eq!(ANTHROPIC_WIRE.read_label(Surface::Assembled), "stream");
    assert_eq!(ANTHROPIC_WIRE.read_label(Surface::Chat), "response");
    assert_eq!(ANTHROPIC_WIRE.read_label(Surface::Incremental), "response");
    assert!(ANTHROPIC_WIRE.strict_error_body(Surface::Chat));
    assert!(!ANTHROPIC_WIRE.strict_error_body(Surface::Assembled));
    assert!(!ANTHROPIC_WIRE.strict_error_body(Surface::Incremental));
    assert!(!ANTHROPIC_WIRE.ends_on_done());
}

/// Anthropic keeps the bare message of a failed send, and only its
/// assembled surface calls the body a stream.
#[tokio::test]
async fn anthropic_keeps_its_send_and_read_error_wording() {
    use crate::domain::shared::error::DomainError;
    use crate::infrastructure::providers::transport::attempt_profile::Profile;
    let error = crate::infrastructure::providers::transport::wire_dialect::refused_send().await;
    let assembled = Profile::new(&ANTHROPIC_WIRE, Surface::Assembled, Default::default());
    assert_eq!(assembled.name(), "Anthropic");
    let DomainError::Provider(sent) = assembled.send_error(&error) else {
        panic!("a provider error");
    };
    assert_eq!(
        sent,
        format!("HTTP error: {error}"),
        "Anthropic keeps the bare message"
    );
    let DomainError::Provider(read) = assembled.read_error(&error) else {
        panic!("a provider error");
    };
    assert_eq!(read, format!("failed to read stream: {error}"));
    let chat = Profile::new(&ANTHROPIC_WIRE, Surface::Chat, Default::default());
    let DomainError::Provider(read) = chat.read_error(&error) else {
        panic!("a provider error");
    };
    assert_eq!(read, format!("failed to read response: {error}"));
    assert!(chat.strict_error_body());
    assert!(!assembled.strict_error_body());
}

/// This wire's cut-short error is one, classified as a retryable network
/// failure (moved from `transport/sse_end_tests.rs`, #2597).
#[test]
fn this_wires_cut_short_error_is_recognised_and_retried() {
    use crate::domain::inference::services::provider_error::{
        ProviderErrorClass, classify_provider_error,
    };
    use crate::domain::shared::error::DomainError;
    use crate::infrastructure::providers::transport::sse_end::is_cut_short;
    let message = super::super::anthropic_sse::ANTHROPIC_CUT_SHORT;
    assert!(is_cut_short(message), "{message}");
    let class = classify_provider_error(&DomainError::Provider(message.into()));
    assert_eq!(class, ProviderErrorClass::Network, "{message}");
}
