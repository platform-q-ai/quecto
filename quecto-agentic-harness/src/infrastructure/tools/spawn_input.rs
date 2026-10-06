//! Tool-input parsing for the `spawn` tool's `container` field.
//!
//! Kept separate from the script-managed adapter so the interface layer never
//! imports adapter modules (argv construction, process execution, JSON
//! contract parsing stay in `spawn_container`).

use crate::domain::environment_registry::EnvironmentTarget;
use crate::domain::subagent::ContainerSelection;
use serde_json::{Map, Value};

pub(super) fn parse_container_selection(
    args: &serde_json::Value,
) -> Result<ContainerSelection, String> {
    let Some(value) = args.get("container") else {
        return Ok(ContainerSelection::Local);
    };
    parse_container_value(value)
}

/// The forms `container` may take once read: `false`, `true` or an object.
/// Only these reach validation, so a string can never be decoded twice.
enum Accepted<'a> {
    Bool(bool),
    Object(&'a Map<String, Value>),
}

impl Accepted<'_> {
    fn kind(&self) -> &'static str {
        match self {
            Self::Bool(_) => "a boolean",
            Self::Object(_) => "an object",
        }
    }
}

/// The refusal for a string that does not hold JSON `true`, `false` or an
/// object.
const STRING_REFUSAL: &str = "container must be false, true, or an object (got a string that is \
     not JSON true, false, or an object; pass the value itself, not a quoted string, e.g. \
     {\"mode\":\"new\"})";

fn parse_container_value(value: &Value) -> Result<ContainerSelection, String> {
    match value {
        Value::String(text) => parse_container_string(text),
        other => match accepted(other) {
            Some(accepted) => select_container(accepted),
            None => Err(type_refusal(other)),
        },
    }
}

/// The accepted form `value` is, if it is one (the one allowlist both the
/// direct and the decoded paths use).
fn accepted(value: &Value) -> Option<Accepted<'_>> {
    match value {
        Value::Bool(selected) => Some(Accepted::Bool(*selected)),
        Value::Object(map) => Some(Accepted::Object(map)),
        Value::Null | Value::Number(_) | Value::String(_) | Value::Array(_) => None,
    }
}

/// The refusal for a value of the wrong JSON type, naming what arrived.
fn type_refusal(value: &Value) -> String {
    let got = match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    };
    format!(
        "container must be false, true, or an object (got {got}; pass false, true, or an object \
         such as {{\"mode\":\"new\"}})"
    )
}

/// Some models send `container` as a quoted JSON string (an
/// anthropic-api/claude-opus-5-5 parent did so when the schema left it
/// untyped, even when told not to). A string holding exactly one JSON
/// `true`, `false` or object, with only JSON whitespace around it, is read
/// as that value and validated as if sent directly; every other string is
/// refused.
fn parse_container_string(text: &str) -> Result<ContainerSelection, String> {
    let Ok(decoded) = serde_json::from_str::<Value>(text) else {
        return Err(STRING_REFUSAL.to_string());
    };
    let Some(accepted) = accepted(&decoded) else {
        return Err(STRING_REFUSAL.to_string());
    };
    tracing::debug!(
        decoded = accepted.kind(),
        "spawn: decoded container sent as a quoted JSON string; validating it"
    );
    select_container(accepted)
}

/// The selection an accepted form names, after the object's field checks.
fn select_container(accepted: Accepted<'_>) -> Result<ContainerSelection, String> {
    match accepted {
        Accepted::Bool(false) => Ok(ContainerSelection::Local),
        Accepted::Bool(true) => Ok(ContainerSelection::New {
            container_config: None,
            name: None,
        }),
        Accepted::Object(map) => parse_container_object(map),
    }
}

fn parse_container_object(
    map: &serde_json::Map<String, serde_json::Value>,
) -> Result<ContainerSelection, String> {
    match map.get("mode").and_then(|v| v.as_str()) {
        Some("new") => parse_new_mode(map),
        Some("existing") => parse_existing_mode(map),
        Some(other) => Err(format!("unsupported container mode '{other}'")),
        None => Err("container.mode is required".to_string()),
    }
}

fn parse_new_mode(
    map: &serde_json::Map<String, serde_json::Value>,
) -> Result<ContainerSelection, String> {
    reject_unknown_container_fields(map, &["mode", "container_config", "name"])?;
    Ok(ContainerSelection::New {
        container_config: optional_string(map, "container_config")?,
        name: optional_string(map, "name")?,
    })
}

fn parse_existing_mode(
    map: &serde_json::Map<String, serde_json::Value>,
) -> Result<ContainerSelection, String> {
    if map.contains_key("container_config") {
        return Err("container.container_config is only valid for mode 'new'".into());
    }
    reject_unknown_container_fields(map, &["mode", "ref", "name"])?;
    let env_ref = optional_string(map, "ref")?;
    let name = optional_string(map, "name")?;
    match (env_ref, name) {
        (Some(env_ref), None) => Ok(ContainerSelection::Existing {
            target: EnvironmentTarget::Ref(env_ref),
        }),
        (None, Some(name)) => Ok(ContainerSelection::Existing {
            target: EnvironmentTarget::Name(name),
        }),
        _ => Err("container mode 'existing' requires exactly one of 'ref' or 'name'".to_string()),
    }
}

fn reject_unknown_container_fields(
    map: &serde_json::Map<String, serde_json::Value>,
    allowed: &[&str],
) -> Result<(), String> {
    if let Some(key) = map.keys().find(|k| !allowed.contains(&k.as_str())) {
        return Err(format!("unknown container field '{key}'"));
    }
    Ok(())
}

fn optional_string(
    map: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Option<String>, String> {
    map.get(key)
        .map(|v| {
            v.as_str()
                .ok_or_else(|| format!("container.{key} must be a string"))
        })
        .transpose()
        .map(|v| v.map(str::to_string))
}

#[cfg(test)]
#[path = "spawn_input_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "spawn_input_slice2_tests.rs"]
mod slice2_tests;

#[cfg(test)]
#[path = "spawn_input_string_tests.rs"]
mod string_tests;
