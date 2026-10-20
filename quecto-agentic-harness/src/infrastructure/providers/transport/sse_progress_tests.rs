//! #2433: the transport's part in which events carry output — `[DONE]` on
//! a wire that ends on it, data that is no JSON, a long event's start read
//! up to its last whole character — on the test wire. Which events of a
//! vendor's wire carry output is that vendor's dialect's (#2597), tested
//! beside it.
use super::{carries_output, first_type, long_event_carries_output};
use crate::infrastructure::providers::transport::attempt_events::Vocabulary;
use crate::infrastructure::providers::transport::wire_dialect::{
    Ending, ErrorName, Generated, TEST_WIRE, WireDialect,
};
use serde_json::Value;

/// The test wire, but not ending on `[DONE]`.
struct NoDone;

impl WireDialect for NoDone {
    fn name(&self) -> &'static str {
        TEST_WIRE.name()
    }
    fn send_error(&self, error: &reqwest::Error) -> String {
        TEST_WIRE.send_error(error)
    }
    fn ends_on_done(&self) -> bool {
        false
    }
    fn vocabulary(&self) -> &'static Vocabulary {
        TEST_WIRE.vocabulary()
    }
    fn error_names(&self) -> &'static [ErrorName] {
        TEST_WIRE.error_names()
    }
    fn error_objects<'a>(&self, value: &'a Value) -> Vec<&'a Value> {
        TEST_WIRE.error_objects(value)
    }
    fn generated(&self, value: &Value) -> Generated {
        TEST_WIRE.generated(value)
    }
    fn output_bytes(&self, value: &Value) -> u64 {
        TEST_WIRE.output_bytes(value)
    }
    fn carries_output(&self, value: &Value) -> bool {
        TEST_WIRE.carries_output(value)
    }
    fn long_event_carries_output(&self, start: &str) -> bool {
        TEST_WIRE.long_event_carries_output(start)
    }
    fn ending<'a>(&self, named: &str, parsed: Option<&'a Value>) -> Ending<'a> {
        TEST_WIRE.ending(named, parsed)
    }
}

#[test]
fn an_event_carries_output_as_its_wire_says() {
    for data in [r#"{"type":"output","text":"a"}"#, r#"{"type":"end"}"#] {
        assert!(carries_output(&TEST_WIRE, data), "{data}");
    }
    for data in [
        r#"{"type":"working"}"#,
        r#"{"type":"made.up","text":"a"}"#,
        r#"{"text":"a"}"#,
        "{not json",
        "1",
        "",
    ] {
        assert!(!carries_output(&TEST_WIRE, data), "{data}");
    }
}

/// `[DONE]` ends the reply, which is output, only on a wire that ends on
/// it (#2597): on any other it is data its dialect reads, and no JSON.
#[test]
fn done_is_output_only_on_a_wire_that_ends_on_it() {
    for done in ["[DONE]", " [DONE] "] {
        assert!(carries_output(&TEST_WIRE, done), "{done:?}");
        assert!(!carries_output(&NoDone, done), "{done:?}");
    }
}

/// A long event is judged by its wire from its kept start, read up to its
/// last whole character (#2433 review round 3).
#[test]
fn a_long_event_is_judged_by_its_wire_from_its_start() {
    assert!(long_event_carries_output(
        &TEST_WIRE,
        br#"{"type":"output","text":"xx"#
    ));
    assert!(long_event_carries_output(
        &TEST_WIRE,
        br#"{"type" : "end", "pad"#
    ));
    assert!(!long_event_carries_output(
        &TEST_WIRE,
        br#"{"type":"working","pad":"xx"#
    ));
    assert!(!long_event_carries_output(&TEST_WIRE, br#"{"padding":"xx"#));
    assert!(!long_event_carries_output(&TEST_WIRE, b""));
    let cut = "{\"type\":\"output\",\"text\":\"é".as_bytes();
    assert!(long_event_carries_output(&TEST_WIRE, &cut[..cut.len() - 1]));
    let invalid = [br#"{"type":"output","text":""#.as_slice(), &[0xff, b'x']].concat();
    assert!(long_event_carries_output(&TEST_WIRE, &invalid));
}

/// The first `"type"` a start names, however it is spaced; none when it
/// names none or its value is no string.
#[test]
fn the_first_type_is_the_first_type_string_named() {
    assert_eq!(first_type(r#"{"type":"a","x":{"type":"b"}}"#), Some("a"));
    assert_eq!(first_type(r#"{"x":{"type" :  "b""#), Some("b"));
    assert_eq!(first_type(r#"{"type":"cut"#), Some("cut"));
    assert_eq!(first_type(r#"{"type":1}"#), None);
    assert_eq!(first_type(r#"{"type""#), None);
    assert_eq!(first_type(r#"{"kind":"a"}"#), None);
}
