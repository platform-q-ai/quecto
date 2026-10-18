use super::*;
use std::fmt;

fn assert_string_round_trip<T>(value: &str)
where
    T: From<String> + serde::Serialize + for<'de> serde::Deserialize<'de> + PartialEq + fmt::Debug,
{
    let id = T::from(value.to_string());
    let json = serde_json::to_string(&id).expect("id serializes as JSON");
    assert_eq!(json, format!("\"{value}\""));
    let decoded: T = serde_json::from_str(&json).expect("id deserializes from JSON string");
    assert_eq!(decoded, id);
}

fn assert_accessors<T>(value: &str)
where
    T: From<String> + From<&'static str> + AsRef<str> + fmt::Display + Clone,
{
    let owned = T::from(value.to_string());
    assert_eq!(owned.as_ref(), value);
    assert_eq!(owned.to_string(), value);
    assert_eq!(T::from("literal").as_ref(), "literal");
}

#[test]
fn identifier_newtypes_are_string_wire_compatible() {
    for value in ["", "worker-1"] {
        assert_string_round_trip::<AgentId>(value);
        assert_accessors::<AgentId>(value);
        assert_eq!(AgentId::new(value).into_string(), value);
    }
    for value in ["", "00000000-0000-0000-0000-000000000001"] {
        assert_string_round_trip::<MessageId>(value);
        assert_accessors::<MessageId>(value);
        assert_eq!(MessageId::new(value).into_string(), value);
    }
    for value in ["", "call_large-1"] {
        assert_string_round_trip::<ToolCallId>(value);
        assert_accessors::<ToolCallId>(value);
        assert_eq!(ToolCallId::new(value).into_string(), value);
    }
    for value in ["", "request-42"] {
        assert_string_round_trip::<CommandId>(value);
        assert_accessors::<CommandId>(value);
        assert_eq!(CommandId::new(value).into_string(), value);
    }
}

#[test]
fn agent_uuid_is_string_wire_compatible() {
    let value = "00000000-0000-4000-8000-000000000001";
    assert_string_round_trip::<AgentUuid>(value);
    assert_accessors::<AgentUuid>(value);
    assert_eq!(AgentUuid::new(value).into_string(), value);
}

#[test]
fn agent_uuid_mint_is_unique_uuid_string() {
    let first = AgentUuid::mint();
    let second = AgentUuid::mint();

    assert_ne!(first, second);
    uuid::Uuid::parse_str(first.as_str()).expect("first minted UUID parses");
    uuid::Uuid::parse_str(second.as_str()).expect("second minted UUID parses");
}

/// #2192 review M2: only the form this harness mints names a child's
/// session when another agent reports it — a lowercase hyphenated uuid —
/// never an arbitrary name such as another session's.
#[test]
fn only_a_minted_form_uuid_is_canonical() {
    assert!(AgentUuid::mint().is_canonical());
    assert!(AgentUuid::new("0f8fad5b-d9cb-469f-a165-70867728950e").is_canonical());
    for other in [
        "secret-plan",
        "",
        "0F8FAD5B-D9CB-469F-A165-70867728950E",
        "0f8fad5bd9cb469fa16570867728950e",
        "{0f8fad5b-d9cb-469f-a165-70867728950e}",
        "urn:uuid:0f8fad5b-d9cb-469f-a165-70867728950e",
        "0f8fad5b-d9cb-469f-a165-70867728950e ",
    ] {
        assert!(!AgentUuid::new(other).is_canonical(), "{other:?}");
    }
}
