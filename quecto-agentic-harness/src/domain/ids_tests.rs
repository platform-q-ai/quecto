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
