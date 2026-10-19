use super::KNOWN_EVENTS;

/// #2433: an event is counted under its name when the harness knows it,
/// and as `(unknown)` otherwise (#2597: each wire names its events; the
/// vendors' own cases live beside their dialects).
#[test]
fn an_event_is_counted_under_a_known_name_or_unknown() {
    let kind = super::named_kind;
    assert_eq!(kind("response.in_progress"), "response.in_progress");
    assert_eq!(kind("response.completed"), "response.completed");
    assert_eq!(kind("ping"), "ping");
    assert_eq!(kind("made.up"), "(unknown)");
    assert_eq!(kind(""), "(unknown)");
}

/// A record keeps the 16 most frequent event types, ties by name.
#[test]
fn a_record_keeps_the_most_frequent_event_types() {
    use crate::domain::inference::value_objects::attempt_diagnostics::{
        EventTypeCounts, MAX_EVENT_TYPES,
    };
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
