//! `container` sent as a quoted JSON string: a string holding JSON `true`,
//! `false` or an object is read as that value and validated the same way;
//! any other string, and every other non-matching type, is refused with a
//! message naming what arrived and how to send it.

use super::*;
use serde_json::{Value, json};

fn parse(container: Value) -> Result<ContainerSelection, String> {
    parse_container_selection(&json!({ "container": container }))
}

fn new_container(container_config: Option<&str>, name: Option<&str>) -> ContainerSelection {
    ContainerSelection::New {
        container_config: container_config.map(str::to_string),
        name: name.map(str::to_string),
    }
}

/// The exact `container` values anthropic-api/claude-opus-5-5 and
/// fireworks glm-5p3-flash parents sent, each refused before this fix.
#[test]
fn the_string_forms_a_parent_sent_now_parse() {
    assert_eq!(
        parse(json!(
            "{\"mode\":\"new\",\"container_config\":\"quecto\",\"name\":\"l1-red-2359\"}"
        )),
        Ok(new_container(Some("quecto"), Some("l1-red-2359")))
    );
    assert_eq!(
        parse(json!(
            "{\"mode\": \"new\", \"container_config\": \"quecto\", \"name\": \"l1-red-2359\"}"
        )),
        Ok(new_container(Some("quecto"), Some("l1-red-2359")))
    );
    // Sent after the owner asked for "a JSON object, not a string".
    assert_eq!(
        parse(json!(
            "{\"mode\": \"new\", \"container_config\": \"quecto\"}"
        )),
        Ok(new_container(Some("quecto"), None))
    );
    assert_eq!(
        parse(json!("{\"container_config\":\"quecto\",\"mode\":\"new\"}")),
        Ok(new_container(Some("quecto"), None))
    );
    assert_eq!(parse(json!("true")), Ok(new_container(None, None)));
    assert_eq!(parse(json!("false")), Ok(ContainerSelection::Local));
    // fireworks/accounts/fireworks/models/glm-5p3-flash quoted it too.
    assert_eq!(
        parse(json!(
            "{\"mode\": \"new\", \"container_config\": \"quecto\", \"name\": \"spike-1839\"}"
        )),
        Ok(new_container(Some("quecto"), Some("spike-1839")))
    );
}

#[test]
fn surrounding_json_whitespace_is_ignored() {
    assert_eq!(parse(json!(" true\n")), Ok(new_container(None, None)));
    assert_eq!(parse(json!("\tfalse ")), Ok(ContainerSelection::Local));
    assert_eq!(parse(json!("\r\ntrue")), Ok(new_container(None, None)));
    assert_eq!(
        parse(json!("\n {\"mode\":\"existing\",\"ref\":\"C1\"} \n")),
        Ok(ContainerSelection::Existing {
            target: EnvironmentTarget::Ref("C1".to_string()),
        })
    );
}

#[test]
fn a_string_holding_an_object_gets_the_same_field_checks() {
    for (container, expected) in [
        (
            "{\"mode\":\"new\",\"repo\":\"r\"}",
            "unknown container field 'repo'",
        ),
        (
            "{\"mode\":\"existing\"}",
            "container mode 'existing' requires exactly one of 'ref' or 'name'",
        ),
        ("{\"mode\":\"other\"}", "unsupported container mode 'other'"),
        ("{}", "container.mode is required"),
        (
            "{\"mode\":\"new\",\"name\":1}",
            "container.name must be a string",
        ),
        (
            "{\"mode\":\"existing\",\"ref\":\"C1\",\"container_config\":\"q\"}",
            "container.container_config is only valid for mode 'new'",
        ),
    ] {
        assert_eq!(
            parse(json!(container)),
            Err(expected.to_string()),
            "{container}"
        );
    }
}

const STRING_REFUSAL: &str = "container must be false, true, or an object (got a string that is \
     not JSON true, false, or an object; pass the value itself, not a quoted string, e.g. \
     {\"mode\":\"new\"})";

#[test]
fn any_other_string_is_refused_with_a_fix() {
    for garbage in [
        "new",
        "yes",
        "",
        "   ",
        "1",
        "null",
        "[true]",
        "\"true\"",
        "\"{\\\"mode\\\":\\\"new\\\"}\"",
        "{\"mode\":\"new\"",
        "true false",
        // Only JSON whitespace may surround the value.
        "\u{a0}true",
        "true\u{2028}",
        "\u{feff}{\"mode\":\"new\"}",
    ] {
        assert_eq!(
            parse(json!(garbage)),
            Err(STRING_REFUSAL.to_string()),
            "{garbage:?}"
        );
    }
}

#[test]
fn other_types_are_refused_naming_what_arrived() {
    for (container, got) in [
        (json!(1), "a number"),
        (json!(1.5), "a number"),
        (json!([true]), "an array"),
        (json!([]), "an array"),
        (json!(null), "null"),
    ] {
        assert_eq!(
            parse(container.clone()),
            Err(format!(
                "container must be false, true, or an object (got {got}; pass false, true, or \
                 an object such as {{\"mode\":\"new\"}})"
            )),
            "{container}"
        );
    }
}

#[test]
fn only_booleans_and_objects_are_accepted_forms() {
    assert_eq!(accepted(&json!(true)).map(|a| a.kind()), Some("a boolean"));
    assert_eq!(accepted(&json!(false)).map(|a| a.kind()), Some("a boolean"));
    assert_eq!(accepted(&json!({})).map(|a| a.kind()), Some("an object"));
    for refused in [json!(null), json!(1), json!("true"), json!([true])] {
        assert!(accepted(&refused).is_none(), "{refused}");
    }
}
