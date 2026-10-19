//! The Responses (Codex) dialect (#2597): the output each event carries,
//! the names its events are counted under, and the events its handler
//! consumes — moved out of `transport/` with the vendor-neutral contract.
use super::*;
use crate::infrastructure::providers::transport::attempt_transport::attempt_events::KNOWN_EVENTS;
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
        assert!(KNOWN_EVENTS.contains(kind), "{kind}");
    }
}

/// Every event type the Codex stream handler matches on is known, so the
/// diagnostics never count the handler's own traffic as unknown (#2158).
#[test]
fn every_event_the_codex_handler_consumes_is_known() {
    let terminal = [
        "response.completed",
        "response.failed",
        "response.incomplete",
    ];
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
                KNOWN_EVENTS.contains(&event) || terminal.contains(&event),
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
