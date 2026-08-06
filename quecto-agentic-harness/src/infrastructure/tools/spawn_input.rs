//! Tool-input parsing for the `spawn` tool's `container` field.
//!
//! Kept separate from the script-managed adapter so the interface layer never
//! imports adapter modules (argv construction, process execution, JSON
//! contract parsing stay in `spawn_container`).

use crate::domain::environment_registry::EnvironmentTarget;
use crate::domain::subagent::ContainerSelection;

pub(super) fn parse_container_selection(
    args: &serde_json::Value,
) -> Result<ContainerSelection, String> {
    let Some(value) = args.get("container") else {
        return Ok(ContainerSelection::Local);
    };
    parse_container_value(value)
}

fn parse_container_value(value: &serde_json::Value) -> Result<ContainerSelection, String> {
    match value {
        serde_json::Value::Bool(false) => Ok(ContainerSelection::Local),
        serde_json::Value::Bool(true) => Ok(ContainerSelection::New {
            repo: None,
            container_script: None,
            name: None,
        }),
        serde_json::Value::Object(map) => parse_container_object(map),
        _ => Err("container must be false, true, or an object".to_string()),
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
    reject_unknown_container_fields(map, &["mode", "repo", "container_script", "name"])?;
    Ok(ContainerSelection::New {
        repo: optional_string(map, "repo")?,
        container_script: optional_string(map, "container_script")?,
        name: optional_string(map, "name")?,
    })
}

fn parse_existing_mode(
    map: &serde_json::Map<String, serde_json::Value>,
) -> Result<ContainerSelection, String> {
    for new_only in ["repo", "container_script"] {
        if map.contains_key(new_only) {
            return Err(format!("container.{new_only} is only valid for mode 'new'"));
        }
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
