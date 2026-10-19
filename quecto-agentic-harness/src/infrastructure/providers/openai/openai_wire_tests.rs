//! The OpenAI chat dialect (#2597): the output each chunk carries, the
//! names chunks are counted under, and its error wording — moved out of
//! `transport/` with the vendor-neutral contract.
use super::*;
use crate::infrastructure::providers::transport::attempt_profile::Surface;
use serde_json::json;

#[test]
fn an_openai_chunk_counts_content_reasoning_refusal_and_tool_arguments() {
    let event = json!({"choices": [
        {"index": 0, "delta": {"content": "ab", "reasoning": "cde", "reasoning_content": "f",
                               "refusal": "gh"}},
        {"index": 1, "delta": {"tool_calls": [
            {"index": 0, "function": {"name": "grep", "arguments": "{\"q\":"}},
            {"index": 1, "function": {"arguments": "1}"}}
        ]}}
    ]});
    // `reasoning` is the reasoning; `reasoning_content` its alternative name.
    assert_eq!(OPENAI_WIRE.output_bytes(&event), 2 + 3 + 2 + 5 + 2);
}

/// #2210 review: the reasoning is counted once, under whichever name the
/// provider streams it, as the handler reads it.
#[test]
fn openai_reasoning_is_counted_under_either_name_once() {
    let cases = [
        (json!({"reasoning": "abcd"}), 4),
        (json!({"reasoning_content": "abc"}), 3),
        (json!({"reasoning": "ab", "reasoning_content": "abcdef"}), 2),
        (json!({"reasoning": null, "reasoning_content": "abcdef"}), 0),
    ];
    for (delta, bytes) in cases {
        let event = json!({"choices": [{"index": 0, "delta": delta}]});
        assert_eq!(OPENAI_WIRE.output_bytes(&event), bytes, "{event}");
    }
}

#[test]
fn an_openai_chunk_without_output_counts_nothing() {
    for event in [
        json!({"choices": [{"index": 0, "delta": {"role": "assistant"}}]}),
        json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}], "usage": {"completion_tokens": 9}}),
        json!({"choices": "none"}),
        json!({"error": {"message": "boom"}}),
    ] {
        assert_eq!(OPENAI_WIRE.output_bytes(&event), 0, "{event}");
    }
}

/// #2433: an OpenAI chat chunk has no type; it is counted under what its
/// first choice carries.
#[test]
fn an_openai_chunk_is_counted_under_what_its_first_choice_carries() {
    let chat = |delta: serde_json::Value, finish: serde_json::Value| json!({"choices": [{"index": 0, "delta": delta, "finish_reason": finish}]});
    for (delta, finish, expected) in [
        (json!({"content": "a"}), json!(null), "chat.delta.content"),
        (json!({"content": ""}), json!(null), "chat.empty"),
        (
            json!({"reasoning_content": "r"}),
            json!(null),
            "chat.delta.reasoning",
        ),
        (
            json!({"tool_calls": [{}]}),
            json!(null),
            "chat.delta.tool_calls",
        ),
        (json!({"refusal": "no"}), json!(null), "chat.delta.refusal"),
        (json!({}), json!("stop"), "chat.finish"),
        (json!({}), json!(null), "chat.empty"),
    ] {
        assert_eq!(OPENAI_WIRE.event_kind("", &chat(delta, finish)), expected);
    }
}

/// The OpenAI wording: only the chat surface is strict about its error
/// body, and a failed read always names the response.
#[test]
fn openai_is_strict_only_on_chat() {
    assert_eq!(OPENAI_WIRE.name(), "OpenAI");
    assert!(OPENAI_WIRE.strict_error_body(Surface::Chat));
    assert!(!OPENAI_WIRE.strict_error_body(Surface::Assembled));
    assert!(!OPENAI_WIRE.strict_error_body(Surface::Incremental));
    for surface in [Surface::Chat, Surface::Assembled, Surface::Incremental] {
        assert_eq!(OPENAI_WIRE.read_label(surface), "response");
    }
    assert!(OPENAI_WIRE.ends_on_done());
}

/// The OpenAI wording of a failed send appends the error's source chain,
/// and a failed read names the response.
#[tokio::test]
async fn openai_keeps_its_send_and_read_error_wording() {
    use crate::domain::shared::error::DomainError;
    use crate::infrastructure::providers::transport::attempt_profile::Profile;
    let error = crate::infrastructure::providers::transport::wire_dialect::refused_send().await;
    let profile = Profile::new(&OPENAI_WIRE, Surface::Chat, Default::default());
    assert_eq!(profile.name(), "OpenAI");
    let DomainError::Provider(sent) = profile.send_error(&error) else {
        panic!("a provider error");
    };
    assert!(sent.starts_with("HTTP error: "), "{sent}");
    assert!(
        sent.len() > format!("HTTP error: {error}").len(),
        "the source chain is appended for OpenAI: {sent}"
    );
    assert!(profile.strict_error_body());
    let DomainError::Provider(read) = profile.read_error(&error) else {
        panic!("a provider error");
    };
    assert_eq!(read, format!("failed to read response: {error}"));
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
    let message = super::super::openai_sse::OPENAI_CUT_SHORT;
    assert!(is_cut_short(message), "{message}");
    let class = classify_provider_error(&DomainError::Provider(message.into()));
    assert_eq!(class, ProviderErrorClass::Network, "{message}");
}

/// An OpenAI reply that ended before any event is the empty stream; one
/// that ended after one keeps its cut-short message.
#[test]
fn an_openai_reply_ended_early_is_empty_or_cut_short() {
    use crate::infrastructure::providers::transport::sse_end::{EMPTY_STREAM, ended_early};
    let cut_short = super::super::openai_sse::OPENAI_CUT_SHORT;
    assert_eq!(ended_early(false, cut_short), EMPTY_STREAM);
    assert_eq!(ended_early(true, cut_short), cut_short);
}

/// #2597: the chat wire knows only its own chunks: another wire's typed
/// event is unknown and ends nothing, and another wire's shapes generate
/// nothing.
#[test]
fn openai_knows_no_other_wires_events() {
    for name in ["ping", "message_stop", "response.completed", "error"] {
        let event = json!({"type": name});
        assert!(!OPENAI_WIRE.known_event(name, &event), "{name}");
        assert_eq!(OPENAI_WIRE.terminal_event(name, &event), None, "{name}");
    }
    for event in [
        json!({"type": "content_block_delta", "delta": {"text": "t", "thinking": "h"}}),
        json!({"type": "content_block_start", "content_block": {"type": "tool_use"}}),
        json!({"type": "response.output_text.delta", "delta": "t"}),
        json!({"type": "response.output_item.added", "item": {"type": "function_call"}}),
    ] {
        assert_eq!(
            OPENAI_WIRE.generated(&event),
            Generated::default(),
            "{event}"
        );
    }
    let chunk = json!({"choices": []});
    assert!(OPENAI_WIRE.known_event("", &chunk));
}

/// What a chat chunk's first choice shows the model generated (#2151).
#[test]
fn an_openai_chunk_shows_text_tool_calls_and_reasoning() {
    let only = |text, tool_call, thinking| Generated {
        text,
        tool_call,
        thinking,
    };
    let chunk = |delta: serde_json::Value| json!({"choices": [{"index": 0, "delta": delta}]});
    for (event, generated) in [
        (chunk(json!({"content": "a"})), only(true, false, false)),
        (chunk(json!({"content": ""})), only(false, false, false)),
        (chunk(json!({"reasoning": "a"})), only(false, false, true)),
        (
            chunk(json!({"reasoning_content": "a"})),
            only(false, false, true),
        ),
        (
            chunk(json!({"tool_calls": [{"index": 0}]})),
            only(false, true, false),
        ),
        (chunk(json!({"tool_calls": []})), only(false, false, false)),
        (chunk(json!({"refusal": "no"})), only(false, false, false)),
    ] {
        assert_eq!(OPENAI_WIRE.generated(&event), generated, "{event}");
    }
}

/// An error chunk (#2155, #2236) is this wire's only terminal event.
#[test]
fn an_openai_error_chunk_records_an_error() {
    use crate::domain::inference::value_objects::attempt_diagnostics::TerminalEvent;
    let chunk = json!({"error": {"message": "boom"}});
    assert_eq!(
        OPENAI_WIRE.terminal_event("", &chunk),
        Some(TerminalEvent::Error)
    );
    let content = json!({"choices": [{"index": 0, "delta": {"content": "a"}}]});
    assert_eq!(OPENAI_WIRE.terminal_event("", &content), None);
}

/// #2433: which chat chunks carry output (moved from `transport/` with the
/// progress vocabulary, #2597).
#[test]
fn chat_chunks_carry_output_by_what_they_hold() {
    let carries = |data: &str| {
        crate::infrastructure::providers::transport::sse_progress::carries_output(
            &OPENAI_WIRE,
            data,
        )
    };
    let chunk = |delta: &str, finish: &str| {
        format!(r#"{{"choices":[{{"index":0,"delta":{delta},"finish_reason":{finish}}}]}}"#)
    };
    for (delta, finish) in [
        (r#"{"content":"a"}"#, "null"),
        (r#"{"reasoning_content":"hm"}"#, "null"),
        (r#"{"reasoning":"hm"}"#, "null"),
        (r#"{"refusal":"no"}"#, "null"),
        (
            r#"{"tool_calls":[{"index":0,"function":{"name":"f"}}]}"#,
            "null",
        ),
        (
            r#"{"tool_calls":[{"index":0,"function":{"arguments":"{"}}]}"#,
            "null",
        ),
        ("{}", r#""stop""#),
    ] {
        assert!(carries(&chunk(delta, finish)), "{delta} {finish}");
    }
    assert!(carries("[DONE]"));
    for (delta, finish) in [
        (r#"{"content":""}"#, "null"),
        (r#"{"role":"assistant"}"#, "null"),
        (r#"{"tool_calls":[]}"#, "null"),
        (r#"{"tool_calls":[{"index":0}]}"#, "null"),
        (
            r#"{"tool_calls":[{"index":0,"function":{"name":"","arguments":""}}]}"#,
            "null",
        ),
        ("{}", "null"),
    ] {
        assert!(!carries(&chunk(delta, finish)), "{delta} {finish}");
    }
    for data in [
        r#"{"choices":[]}"#,
        r#"{"error":{"message":"boom"}}"#,
        "{not json",
        // Another wire's output is none of this wire's (#2597).
        r#"{"type":"response.output_text.delta","delta":"a"}"#,
        r#"{"type":"response.completed","response":{}}"#,
        r#"{"type":"error","message":"x"}"#,
        r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":"a"}}"#,
        r#"{"type":"message_stop"}"#,
    ] {
        assert!(!carries(data), "{data}");
    }
}

/// #2433 review round 3 (N1): a chat chunk too long to read whole is output
/// when it carries content or a tool call, though a `"type"` comes later
/// inside its choices.
#[test]
fn a_long_chat_chunk_is_output_by_what_its_choices_carry() {
    let long = |start: &[u8]| {
        crate::infrastructure::providers::transport::sse_progress::long_event_carries_output(
            &OPENAI_WIRE,
            start,
        )
    };
    let call = format!(
        r#"{{"choices":[{{"index":0,"delta":{{"tool_calls":[{{"index":0,"id":"c","type":"function","function":{{"name":"f","arguments":"{}"#,
        "x".repeat(70 * 1024)
    );
    assert!(long(&call.as_bytes()[..64 * 1024]));
    assert!(long(br#"{"choices":[{"index":0,"delta":{"content":"xx"#));
    assert!(long(
        br#"{"choices":[{"index":0,"delta":{"reasoning_content":"xx"#
    ));
    let padded = r#"{"choices":[{"index":0,"delta":{"role":"assistant","padding":"xxx"#;
    assert!(!long(padded.as_bytes()));
    assert!(!long(br#"{"padding":"xx"#));
    assert!(!long(b""));
    // Another wire's long event is none of this wire's (#2597).
    assert!(!long(
        br#"{"type":"response.output_text.delta","delta":"xxxx"#
    ));
    assert!(!long(
        br#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"xx"#
    ));
    assert!(!long(br#"{"type":"x","content":"xx","choices":[{"xx"#));
}

/// #1679 P2 AC5, #2609 review: a chat payload is a throttle by its `error`
/// object's typed fields alone, a billing name taking precedence. The chat
/// wire is the OpenAI-compatible one, so it keeps the compatible vendors'
/// names (OpenRouter relays an upstream's `overloaded_error`); only where
/// it reads them narrows. Moved from `transport/` and the contract suite.
#[test]
fn a_chat_throttle_is_read_from_its_error_object_only() {
    for (value, throttle) in [
        (json!({"error": {"type": "rate_limit_error"}}), true),
        (json!({"error": {"type": "overloaded_error"}}), true),
        (json!({"error": {"code": "rate_limit_exceeded"}}), true),
        (
            json!({"error": {"type": "rate_limit_error", "code": "insufficient_quota"}}),
            false,
        ),
        (
            json!({"error": {"type": "overloaded_error", "code": "billing_hard_limit_reached"}}),
            false,
        ),
        (
            json!({"error": {"message": "429 rate_limit_error overloaded_error"}}),
            false,
        ),
        (json!({"error": {"type": "authentication_error"}}), false),
        (json!({"error": {"type": "invalid_request_error"}}), false),
        (json!({"error": {"code": 429}}), false),
        (json!(null), false),
        // Declared narrowing: the Responses shapes.
        (
            json!({"type": "error", "code": "rate_limit_exceeded"}),
            false,
        ),
        (
            json!({"response": {"error": {"code": "rate_limit_exceeded"}}}),
            false,
        ),
    ] {
        assert_eq!(OPENAI_WIRE.is_throttle(&value), throttle, "{value}");
    }
}

/// An HTTP error body naming a billing error or a refused request is
/// terminal; a throttle or a server error is not.
#[test]
fn a_chat_error_body_is_terminal_by_its_typed_fields() {
    for (value, terminal) in [
        (json!({"error": {"code": "insufficient_quota"}}), true),
        (json!({"error": {"code": "invalid_api_key"}}), true),
        (json!({"error": {"type": "authentication_error"}}), true),
        (json!({"error": {"type": "permission_error"}}), true),
        (json!({"error": {"type": "invalid_request_error"}}), true),
        (json!({"error": {"code": "rate_limit_exceeded"}}), false),
        (json!({"error": {"code": "server_error"}}), false),
        // Declared narrowing: a Responses-nested error.
        (
            json!({"response": {"error": {"code": "invalid_api_key"}}}),
            false,
        ),
    ] {
        assert_eq!(OPENAI_WIRE.is_terminal_error(&value), terminal, "{value}");
    }
}

/// The chat wire's error names an attempt records: every name the
/// compatible vendors send.
#[test]
fn a_chat_error_records_its_code() {
    use crate::domain::inference::value_objects::attempt_diagnostics::ErrorCode;
    for (name, code) in [
        ("rate_limit_exceeded", ErrorCode::RateLimitExceeded),
        ("rate_limit_error", ErrorCode::RateLimitExceeded),
        ("overloaded_error", ErrorCode::Overloaded),
        ("server_error", ErrorCode::ServerError),
        ("insufficient_quota", ErrorCode::InsufficientQuota),
        ("usage_limit_reached", ErrorCode::UsageLimitReached),
        ("authentication_error", ErrorCode::AuthenticationError),
        ("permission_error", ErrorCode::PermissionError),
        ("invalid_request_error", ErrorCode::InvalidRequestError),
        ("invalid_api_key", ErrorCode::InvalidApiKey),
    ] {
        assert_eq!(
            OPENAI_WIRE.error_name(name).map(|known| known.code),
            Some(code),
            "{name}"
        );
    }
    assert!(OPENAI_WIRE.error_name("SECRET").is_none());
    // OpenAI's 401 names two known errors: its `type`'s is recorded, as
    // before the move (#2620 review).
    let profile = crate::infrastructure::providers::transport::attempt_profile::Profile::new(
        &OPENAI_WIRE,
        Surface::Incremental,
        Default::default(),
    );
    let seen = crate::infrastructure::providers::transport::attempt_transport::observed(
        profile,
        &[r#"data: {"error":{"type":"invalid_request_error","code":"invalid_api_key"}}"#],
        None,
    );
    assert_eq!(
        seen.diagnostics.error_code,
        Some(ErrorCode::InvalidRequestError)
    );
    let value = json!({"response": {"incomplete_details": {"reason": "max_output_tokens"}}});
    assert_eq!(OPENAI_WIRE.incomplete_reason(&value), None);
}
