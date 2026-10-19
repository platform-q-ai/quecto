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
