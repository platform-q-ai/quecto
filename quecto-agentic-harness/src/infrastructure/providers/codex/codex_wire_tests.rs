//! The Responses (Codex) dialect (#2597): the output each event carries,
//! the names its events are counted under, and the events its handler
//! consumes — moved out of `transport/` with the vendor-neutral contract.
use super::*;
use serde_json::json;

#[test]
fn a_codex_delta_counts_only_its_generated_text() {
    for kind in RESPONSES_OUTPUT_DELTAS {
        let event = json!({"type": kind, "delta": "héllo", "item_id": "x", "sequence_number": 7});
        assert_eq!(CODEX_WIRE.output_bytes(&event), 6, "{kind}");
    }
}

#[test]
fn a_codex_event_that_is_no_output_delta_counts_nothing() {
    for event in [
        json!({"type": "response.created", "response": {}}),
        json!({"type": "response.output_text.done", "text": "hello"}),
        json!({"type": "response.function_call_arguments.done", "arguments": "{}"}),
        json!({"type": "response.output_item.added", "item": {"type": "function_call"}}),
        json!({"type": "response.output_text.delta", "delta": 5}),
        json!({"type": "response.audio.delta", "delta": "not model text output"}),
        json!({"type": "response.output_text.annotation.delta", "delta": "x"}),
        json!({"delta": "untyped"}),
    ] {
        assert_eq!(CODEX_WIRE.output_bytes(&event), 0, "{event}");
    }
}

#[test]
fn every_codex_output_delta_is_a_known_event() {
    for kind in RESPONSES_OUTPUT_DELTAS {
        assert!(RESPONSES.known.contains(kind), "{kind}");
    }
}

/// Every event type the Codex stream handler matches on is known, so the
/// diagnostics never count the handler's own traffic as unknown (#2158).
#[test]
fn every_event_the_codex_handler_consumes_is_known() {
    for source in [
        include_str!("mod.rs"),
        include_str!("codex_sse_state.rs"),
        include_str!("codex_sse_handler.rs"),
    ] {
        for event in source
            .split('"')
            .skip(1)
            .step_by(2)
            .filter(|literal| literal.starts_with("response."))
        {
            assert!(
                RESPONSES.name(event).is_some(),
                "{event} is consumed by the Codex handler but not known to diagnostics"
            );
        }
    }
}

/// #2433: a Responses event is counted under its type when the harness
/// knows it, and as `(unknown)` otherwise.
#[test]
fn a_codex_event_is_counted_under_a_known_name_or_unknown() {
    let codex = |kind: &str| json!({"type": kind});
    for (event, expected) in [
        (codex("response.in_progress"), "response.in_progress"),
        (codex("response.completed"), "response.completed"),
        (codex("made.up"), "(unknown)"),
        (json!({}), "(unknown)"),
    ] {
        assert_eq!(CODEX_WIRE.event_kind("", &event), expected, "{event}");
    }
}

/// The Codex wording of a failed send and its error bodies.
#[test]
fn codex_names_itself_and_adds_no_retry_after_note() {
    assert_eq!(CODEX_WIRE.name(), "Codex");
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("retry-after", "7".parse().unwrap());
    assert_eq!(CODEX_WIRE.retry_after(&headers), "");
    for surface in [Surface::Chat, Surface::Assembled, Surface::Incremental] {
        assert!(!CODEX_WIRE.strict_error_body(surface));
        assert_eq!(CODEX_WIRE.read_label(surface), "response");
    }
}

use crate::infrastructure::providers::transport::attempt_profile::Surface;

/// The Codex wording of a failed send, and a failed read names the response.
#[tokio::test]
async fn codex_keeps_its_send_and_read_error_wording() {
    use crate::domain::shared::error::DomainError;
    use crate::infrastructure::providers::transport::attempt_profile::Profile;
    let error = crate::infrastructure::providers::transport::wire_dialect::refused_send().await;
    let profile = Profile::new(&CODEX_WIRE, Surface::Incremental, Default::default());
    assert_eq!(profile.name(), "Codex");
    let DomainError::Provider(sent) = profile.send_error(&error) else {
        panic!("a provider error");
    };
    assert!(sent.starts_with("Codex request failed: "), "{sent}");
    let DomainError::Provider(read) = profile.read_error(&error) else {
        panic!("a provider error");
    };
    assert_eq!(read, format!("failed to read response: {error}"));
    assert!(!Profile::new(&CODEX_WIRE, Surface::Chat, Default::default()).strict_error_body());
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
    let message = super::super::RESPONSES_CUT_SHORT;
    assert!(is_cut_short(message), "{message}");
    let class = classify_provider_error(&DomainError::Provider(message.into()));
    assert_eq!(class, ProviderErrorClass::Network, "{message}");
}

/// #2597: the Responses wire knows only its own events: another wire's
/// event is unknown and ends nothing, and another wire's shapes generate
/// nothing.
#[test]
fn codex_knows_no_other_wires_events() {
    for name in [
        "ping",
        "message_start",
        "message_stop",
        "content_block_delta",
    ] {
        let event = json!({"type": name});
        assert!(!CODEX_WIRE.known_event(name, &event), "{name}");
        assert_eq!(CODEX_WIRE.terminal_event(name, &event), None, "{name}");
        assert_eq!(CODEX_WIRE.event_kind("", &event), "(unknown)", "{name}");
    }
    for event in [
        json!({"choices": [{"delta": {"content": "c", "reasoning": "r", "tool_calls": [{}]}}]}),
        json!({"type": "content_block_delta", "delta": {"text": "t", "thinking": "h"}}),
        json!({"type": "content_block_start", "content_block": {"type": "tool_use"}}),
    ] {
        assert_eq!(
            CODEX_WIRE.generated(&event),
            Generated::default(),
            "{event}"
        );
    }
}

/// What each Responses event shows the model generated (#2151, #2158).
#[test]
fn a_codex_event_shows_text_tool_calls_and_reasoning() {
    let text = Generated {
        text: true,
        ..Generated::default()
    };
    let thinking = Generated {
        thinking: true,
        ..Generated::default()
    };
    let tool_call = Generated {
        tool_call: true,
        ..Generated::default()
    };
    for (event, generated) in [
        (
            json!({"type": "response.output_text.delta", "delta": "a"}),
            text,
        ),
        (
            json!({"type": "response.output_text.delta", "delta": ""}),
            Generated::default(),
        ),
        (
            json!({"type": "response.reasoning_summary_text.delta", "delta": "a"}),
            thinking,
        ),
        (
            json!({"type": "response.reasoning.summary_text.delta", "delta": "a"}),
            thinking,
        ),
        (
            json!({"type": "response.reasoning_summary_text.delta", "delta": ""}),
            Generated::default(),
        ),
        (
            json!({"type": "response.refusal.delta", "delta": "a"}),
            Generated::default(),
        ),
        (
            json!({"type": "response.output_item.added", "item": {"type": "function_call"}}),
            tool_call,
        ),
        (
            json!({"type": "response.output_item.added", "item": {"type": "message"}}),
            Generated::default(),
        ),
    ] {
        assert_eq!(CODEX_WIRE.generated(&event), generated, "{event}");
    }
}

/// The Responses terminal events, and an untyped error chunk (#2249
/// review), record what ended the attempt.
#[test]
fn codex_terminal_events_record_how_the_reply_ended() {
    use crate::domain::inference::value_objects::attempt_diagnostics::TerminalEvent;
    for (name, terminal) in [
        ("response.completed", TerminalEvent::ResponseCompleted),
        ("response.failed", TerminalEvent::ResponseFailed),
        ("response.incomplete", TerminalEvent::ResponseIncomplete),
        ("error", TerminalEvent::Error),
    ] {
        let event = json!({"type": name});
        assert_eq!(
            CODEX_WIRE.terminal_event(name, &event),
            Some(terminal),
            "{name}"
        );
        assert_eq!(CODEX_WIRE.event_kind("", &event), name, "{name}");
    }
    let chunk = json!({"error": {"message": "boom"}});
    assert_eq!(
        CODEX_WIRE.terminal_event("", &chunk),
        Some(TerminalEvent::Error)
    );
    let created = json!({"type": "response.created"});
    assert_eq!(
        CODEX_WIRE.terminal_event("response.created", &created),
        None
    );
}

/// #2433: which Responses events carry output (moved from `transport/` with
/// the progress vocabulary, #2597).
#[test]
fn responses_events_carry_output_by_what_they_hold() {
    let carries = |data: &str| {
        crate::infrastructure::providers::transport::sse_progress::carries_output(&CODEX_WIRE, data)
    };
    let output = [
        r#"{"type":"response.output_text.delta","delta":"a"}"#,
        r#"{"type":"response.function_call_arguments.delta","delta":"{"}"#,
        r#"{"type":"response.refusal.delta","delta":"no"}"#,
        r#"{"type":"response.reasoning_summary_text.delta","delta":"hm"}"#,
        r#"{"type":"response.reasoning_text.delta","delta":"hm"}"#,
        r#"{"type":"response.output_text.done","text":"ab"}"#,
        r#"{"type":"response.function_call_arguments.done","arguments":"{}"}"#,
        r#"{"type":"response.output_item.done","item":{"type":"message","content":[{"type":"output_text","text":"ab"}]}}"#,
        r#"{"type":"response.output_item.done","item":{"type":"reasoning","summary":[{"type":"summary_text","text":"hm"}]}}"#,
        r#"{"type":"response.output_item.done","item":{"type":"function_call","name":"f","arguments":"{}"}}"#,
        r#"{"type":"response.content_part.done","part":{"type":"output_text","text":"ab"}}"#,
        r#"{"type":"response.content_part.done","part":{"type":"refusal","refusal":"no"}}"#,
        r#"{"type":"response.reasoning_summary_part.done","part":{"type":"summary_text","text":"hm"}}"#,
        r#"{"type":"response.completed","response":{}}"#,
        r#"{"type":"response.incomplete","response":{}}"#,
        r#"{"type":"response.failed","response":{}}"#,
        r#"{"type":"error","message":"x"}"#,
        "[DONE]",
    ];
    for data in output {
        assert!(carries(data), "{data}");
    }
    let none = [
        r#"{"type":"response.created","response":{}}"#,
        r#"{"type":"response.in_progress","response":{}}"#,
        r#"{"type":"response.output_text.delta","delta":""}"#,
        r#"{"type":"response.reasoning_summary_text.delta","delta":""}"#,
        r#"{"type":"response.function_call_arguments.delta","delta":""}"#,
        r#"{"type":"response.output_item.added","item":{"type":"message","content":[]}}"#,
        r#"{"type":"response.output_item.added","item":{"type":"reasoning","summary":[]}}"#,
        r#"{"type":"response.output_item.added","item":{"type":"function_call","name":"f","arguments":""}}"#,
        r#"{"type":"response.output_item.done","item":{"type":"reasoning","summary":[],"encrypted_content":"x"}}"#,
        r#"{"type":"response.output_item.done","item":{"type":"message","content":[{"type":"output_text","text":""}]}}"#,
        r#"{"type":"response.content_part.added","part":{"type":"output_text","text":""}}"#,
        r#"{"type":"response.reasoning_summary_part.added","part":{"type":"summary_text","text":""}}"#,
        r#"{"type":"response.output_text.done","text":""}"#,
        r#"{"type":"made.up"}"#,
        r#"{"error":{"message":"untyped"}}"#,
        "{not json",
        "",
        // Another wire's output is none of this wire's (#2597).
        r#"{"type":"message_stop"}"#,
        r#"{"choices":[{"index":0,"delta":{"content":"a"},"finish_reason":"stop"}]}"#,
        r#"{"type":"content_block_start","content_block":{"type":"text","text":"a"}}"#,
    ];
    for data in none {
        assert!(!carries(data), "{data}");
    }
}

/// #2433 review M4: a long Responses event is output by the type its start
/// names: a delta, a finished part or a terminal event.
#[test]
fn a_long_codex_event_is_output_by_the_type_its_start_names() {
    let long = |start: &str| {
        crate::infrastructure::providers::transport::sse_progress::long_event_carries_output(
            &CODEX_WIRE,
            start.as_bytes(),
        )
    };
    for start in [
        r#"{"type":"response.output_text.delta","delta":"xxxx"#,
        r#"{"type":"response.output_item.done","item":{"content":[{"text":"xx"#,
        r#"{"type":"response.completed","response":{"output":[{"xx"#,
        r#"{"type":"error","message":"xx"#,
    ] {
        assert!(long(start), "{start}");
    }
    for start in [
        r#"{"type":"response.in_progress","response":{"padding":"xx"#,
        r#"{"type":"response.created","response":{"xx"#,
        r#"{"padding":"xx"#,
        "",
        // Another wire's long event is none of this wire's (#2597).
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"xx"#,
        r#"{"choices":[{"index":0,"delta":{"content":"xx"#,
    ] {
        assert!(!long(start), "{start}");
    }
}

/// #2609 review round 1: full reasoning text (the OpenAI SDK's
/// `ResponseReasoningText{Delta,Done}Event`) stays unknown until the Codex
/// handler consumes it, so the #2158 unknown-event count shows it arriving.
#[test]
fn codex_reasoning_text_stays_unknown_until_handled() {
    for name in [
        "response.reasoning_text.delta",
        "response.reasoning_text.done",
    ] {
        let event = json!({"type": name, "delta": "hm"});
        assert!(!CODEX_WIRE.known_event(name, &event), "{name}");
        assert_eq!(CODEX_WIRE.event_kind("", &event), "(unknown)");
    }
}
