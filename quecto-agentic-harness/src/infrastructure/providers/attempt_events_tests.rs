use super::KNOWN_EVENTS;

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
        include_str!("codex.rs"),
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

/// #2433: events are counted under names the harness knows; any other is
/// `(unknown)`, so a record holds no name a provider made up.
#[test]
fn an_event_is_counted_under_a_known_name_or_unknown() {
    use super::super::super::attempt_profile::Vendor;
    use serde_json::json;
    let kind = super::event_kind;
    let codex = |kind_name: &str| json!({"type": kind_name});
    assert_eq!(
        kind(Vendor::Codex, "", &codex("response.in_progress")),
        "response.in_progress"
    );
    assert_eq!(
        kind(Vendor::Codex, "", &codex("response.completed")),
        "response.completed"
    );
    assert_eq!(kind(Vendor::Codex, "", &codex("made.up")), "(unknown)");
    assert_eq!(kind(Vendor::Codex, "", &json!({})), "(unknown)");
    assert_eq!(kind(Vendor::Anthropic, "ping", &json!({})), "ping");
    assert_eq!(kind(Vendor::Anthropic, "", &json!({})), "(unknown)");
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
        assert_eq!(kind(Vendor::OpenAi, "", &chat(delta, finish)), expected);
    }
}

/// A record keeps the 16 most frequent event types, ties by name.
#[test]
fn a_record_keeps_the_most_frequent_event_types() {
    use crate::domain::attempt_diagnostics::{EventTypeCounts, MAX_EVENT_TYPES};
    let mut counts = EventTypeCounts::default();
    for (rank, name) in KNOWN_EVENTS.iter().enumerate() {
        for _ in 0..=rank {
            counts.count(name);
        }
    }
    assert!(KNOWN_EVENTS.len() > MAX_EVENT_TYPES);
    let top = counts.top();
    let kept = KNOWN_EVENTS.iter().rev().take(MAX_EVENT_TYPES);
    for (rank, name) in kept.enumerate() {
        assert_eq!(top.get(name), (KNOWN_EVENTS.len() - rank) as u32, "{name}");
    }
    assert_eq!(top.get(KNOWN_EVENTS[0]), 0, "the rarest is dropped");
    let encoded = serde_json::to_value(&top).unwrap();
    assert_eq!(encoded.as_object().unwrap().len(), MAX_EVENT_TYPES);
}
