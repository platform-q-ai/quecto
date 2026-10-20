use super::{UNKNOWN_KIND, Vocabulary};
use crate::domain::inference::value_objects::attempt_diagnostics::TerminalEvent;

/// A wire of two known events and two terminal ones.
const WIRE: Vocabulary = Vocabulary {
    known: &["start", "part.delta"],
    terminal: &[
        ("end", TerminalEvent::Done),
        ("fault", TerminalEvent::Error),
    ],
};

/// #2433: an event is counted under its name when the wire knows it, and
/// as `(unknown)` otherwise (#2597: each wire names its own events; the
/// vendors' cases live beside their dialects).
#[test]
fn an_event_is_counted_under_a_known_name_or_unknown() {
    assert_eq!(WIRE.kind("start"), "start");
    assert_eq!(WIRE.kind("part.delta"), "part.delta");
    assert_eq!(WIRE.kind("end"), "end");
    assert_eq!(WIRE.kind("made.up"), UNKNOWN_KIND);
    assert_eq!(WIRE.kind(""), UNKNOWN_KIND);
    assert_eq!(WIRE.kind("START"), UNKNOWN_KIND, "names match exactly");
}

/// A terminal event records what its wire says; nothing else is terminal.
#[test]
fn only_a_terminal_name_records_a_terminal_event() {
    assert_eq!(WIRE.terminal("end"), Some(TerminalEvent::Done));
    assert_eq!(WIRE.terminal("fault"), Some(TerminalEvent::Error));
    for other in ["start", "part.delta", "", "made.up"] {
        assert_eq!(WIRE.terminal(other), None, "{other}");
    }
    assert_eq!(WIRE.name("fault"), Some("fault"));
    assert_eq!(WIRE.name("made.up"), None);
}

/// A record keeps the 16 most frequent event types, ties by name.
#[test]
fn a_record_keeps_the_most_frequent_event_types() {
    use crate::domain::inference::value_objects::attempt_diagnostics::{
        EventTypeCounts, MAX_EVENT_TYPES,
    };
    let names: Vec<&'static str> = (0..=MAX_EVENT_TYPES)
        .map(|n| &*Box::leak(format!("event.{n:02}").into_boxed_str()))
        .collect();
    let mut counts = EventTypeCounts::default();
    for (rank, name) in names.iter().enumerate() {
        for _ in 0..=rank {
            counts.count(name);
        }
    }
    assert!(names.len() > MAX_EVENT_TYPES);
    let top = counts.top();
    let kept = names.iter().rev().take(MAX_EVENT_TYPES);
    for (rank, name) in kept.enumerate() {
        assert_eq!(top.get(name), (names.len() - rank) as u32, "{name}");
    }
    assert_eq!(top.get(names[0]), 0, "the rarest is dropped");
    let encoded = serde_json::to_value(&top).unwrap();
    assert_eq!(encoded.as_object().unwrap().len(), MAX_EVENT_TYPES);
}
