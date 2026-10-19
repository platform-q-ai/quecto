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

/// #2597: the Messages wire knows only its own events: another wire's
/// event line names nothing, and another wire's shapes generate nothing.
#[test]
fn anthropic_knows_no_other_wires_events() {
    for name in ["response.completed", "response.created", "[DONE]"] {
        assert_eq!(ANTHROPIC_WIRE.event_line(name), Some(""), "{name}");
        assert!(!ANTHROPIC_WIRE.known_event(name, &json!({})), "{name}");
        assert_eq!(
            ANTHROPIC_WIRE.terminal_event(name, &json!({})),
            None,
            "{name}"
        );
    }
    for event in [
        json!({"choices": [{"delta": {"content": "c", "reasoning": "r", "tool_calls": [{}]}}]}),
        json!({"type": "response.output_text.delta", "delta": "t"}),
        json!({"type": "response.reasoning_summary_text.delta", "delta": "t"}),
        json!({"type": "response.output_item.added", "item": {"type": "function_call"}}),
    ] {
        assert_eq!(
            ANTHROPIC_WIRE.generated(&event),
            Generated::default(),
            "{event}"
        );
    }
}

/// What each Messages event shows the model generated.
#[test]
fn an_anthropic_event_shows_text_tool_calls_and_thinking() {
    let only = |text, tool_call, thinking| Generated {
        text,
        tool_call,
        thinking,
    };
    for (event, generated) in [
        (
            json!({"type": "content_block_delta", "delta": {"type": "text_delta", "text": "a"}}),
            only(true, false, false),
        ),
        (
            json!({"type": "content_block_delta", "delta": {"type": "text_delta", "text": ""}}),
            only(false, false, false),
        ),
        (
            json!({"type": "content_block_delta", "delta": {"type": "thinking_delta", "thinking": "a"}}),
            only(false, false, true),
        ),
        (
            json!({"type": "content_block_start", "content_block": {"type": "tool_use", "name": "f"}}),
            only(false, true, false),
        ),
        (
            json!({"type": "content_block_start", "content_block": {"type": "text", "text": ""}}),
            only(false, false, false),
        ),
        (
            json!({"type": "content_block_delta", "delta": {"type": "input_json_delta", "partial_json": "{"}}),
            only(false, false, false),
        ),
    ] {
        assert_eq!(ANTHROPIC_WIRE.generated(&event), generated, "{event}");
    }
}

/// The Messages terminal events record what ended the attempt.
#[test]
fn anthropic_terminal_events_record_how_the_reply_ended() {
    use crate::domain::inference::value_objects::attempt_diagnostics::TerminalEvent;
    let value = json!({});
    assert_eq!(
        ANTHROPIC_WIRE.terminal_event("error", &value),
        Some(TerminalEvent::Error)
    );
    assert_eq!(
        ANTHROPIC_WIRE.terminal_event("message_stop", &value),
        Some(TerminalEvent::MessageStop)
    );
    for name in ["ping", "message_delta", ""] {
        assert_eq!(ANTHROPIC_WIRE.terminal_event(name, &value), None, "{name}");
    }
}

/// #2433: which Messages events carry output (moved from `transport/` with
/// the progress vocabulary, #2597). `[DONE]` is none: this wire ends at
/// `message_stop`.
#[test]
fn messages_events_carry_output_by_what_they_hold() {
    let carries = |data: &str| {
        crate::infrastructure::providers::transport::sse_progress::carries_output(
            &ANTHROPIC_WIRE,
            data,
        )
    };
    for data in [
        r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":"a"}}"#,
        r#"{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"hm"}}"#,
        r#"{"type":"content_block_delta","delta":{"type":"input_json_delta","partial_json":"{"}}"#,
        r#"{"type":"content_block_start","content_block":{"type":"text","text":"a"}}"#,
        r#"{"type":"content_block_start","content_block":{"type":"thinking","thinking":"hm"}}"#,
        r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#,
        r#"{"type":"message_stop"}"#,
        r#"{"type":"error","error":{"type":"overloaded_error"}}"#,
    ] {
        assert!(carries(data), "{data}");
    }
    for data in [
        r#"{"type":"ping"}"#,
        r#"{"type":"message_start","message":{"content":[]}}"#,
        r#"{"type":"content_block_start","content_block":{"type":"text","text":""}}"#,
        r#"{"type":"content_block_start","content_block":{"type":"tool_use","name":"f","input":{}}}"#,
        r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":""}}"#,
        r#"{"type":"content_block_delta","delta":{"type":"signature_delta","signature":"s"}}"#,
        r#"{"type":"content_block_stop","index":0}"#,
        r#"{"type":"message_delta","delta":{"stop_reason":null},"usage":{}}"#,
        "[DONE]",
        "{not json",
        // Another wire's output is none of this wire's (#2597).
        r#"{"type":"response.completed","response":{}}"#,
        r#"{"choices":[{"index":0,"delta":{"content":"a"},"finish_reason":"stop"}]}"#,
    ] {
        assert!(!carries(data), "{data}");
    }
}

/// #2433 review M4: a long Messages event is output by the type its start
/// names: a delta or a terminal event.
#[test]
fn a_long_anthropic_event_is_output_by_the_type_its_start_names() {
    let long = |start: &str| {
        crate::infrastructure::providers::transport::sse_progress::long_event_carries_output(
            &ANTHROPIC_WIRE,
            start.as_bytes(),
        )
    };
    for start in [
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"xx"#,
        r#"{"index":0,"delta":{"type":"text_delta","text":"xx"#,
        r#"{"type":"message_stop","padding":"xx"#,
    ] {
        assert!(long(start), "{start}");
    }
    for start in [
        r#"{"type": "ping", "padding":"xx"#,
        r#"{"type":"message_start","message":{"xx"#,
        r#"{"padding":"xx"#,
        "",
        // Another wire's long event is none of this wire's (#2597).
        r#"{"type":"response.output_text.delta","delta":"xxxx"#,
        r#"{"choices":[{"index":0,"delta":{"content":"xx"#,
    ] {
        assert!(!long(start), "{start}");
    }
}
