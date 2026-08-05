//! Tool-input parsing for the `spawn` tool's `container` field.
//!
//! Kept separate from the script-managed adapter so the interface layer never
//! imports adapter modules (argv construction, process execution, JSON
//! contract parsing stay in `spawn_container`).

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
        }),
        serde_json::Value::Object(map) => parse_container_object(map),
        _ => Err("container must be false, true, or an object".to_string()),
    }
}

fn parse_container_object(
    map: &serde_json::Map<String, serde_json::Value>,
) -> Result<ContainerSelection, String> {
    reject_unknown_container_fields(map)?;
    require_new_mode(map)?;
    Ok(ContainerSelection::New {
        repo: optional_string(map, "repo")?,
        container_script: optional_string(map, "container_script")?,
    })
}

fn reject_unknown_container_fields(
    map: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), String> {
    let allowed = ["mode", "repo", "container_script"];
    if let Some(key) = map.keys().find(|k| !allowed.contains(&k.as_str())) {
        return Err(format!("unknown container field '{key}'"));
    }
    Ok(())
}

fn require_new_mode(map: &serde_json::Map<String, serde_json::Value>) -> Result<(), String> {
    match map.get("mode").and_then(|v| v.as_str()) {
        Some("new") => Ok(()),
        Some("existing") => {
            Err("container mode 'existing' is not supported in this slice".to_string())
        }
        Some(other) => Err(format!("unsupported container mode '{other}'")),
        None => Err("container.mode is required".to_string()),
    }
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
