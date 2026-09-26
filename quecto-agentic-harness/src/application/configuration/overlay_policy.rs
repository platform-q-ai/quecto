//! The overlay policy (#2024): which sections a repo-local overlay may
//! carry, how each merges over the global document, and the dotted-path
//! access the read and patch use cases share.
//!
//! Merge rules, per top-level section of the overlay:
//! - `agents` — `agents.defaults` field-wise: an overlay field replaces the
//!   global field, other global fields stay;
//! - `tools` — `tools.web` field-wise per engine, `tools.grep` field-wise
//!   per section (`relevance`, `log`; #2136), `tools.policy.entries`
//!   entry-wise (an overlay entry replaces the global entry of the same
//!   stable id), anything else under `tools` replaced whole;
//! - `container_configs` — entry-wise; an overlay entry labelled
//!   `"default": true` un-defaults every global entry;
//! - `workflow` — field-wise (`templates` replaced whole);
//! - `providers`, `admission` — global-only: an overlay carrying them is
//!   refused naming the key; so is an overlay turning `tools.grep.relevance`
//!   on or `tools.grep.log` off (#2136; see [`GLOBAL_ONLY_PATHS`]);
//! - any other key — replaced whole (unknown keys pass through both files).

use serde_json::{Map, Value};

/// Sections only the global file may define.
pub const GLOBAL_ONLY_KEYS: &[&str] = &["providers", "admission"];

/// Every top-level section the configuration schema knows. A file that
/// carries none of them is not a quecto configuration.
pub const KNOWN_TOP_LEVEL_KEYS: &[&str] = &[
    "agents",
    "providers",
    "tools",
    "workflow",
    "container_configs",
    "admission",
    "telemetry",
];

/// Whether `document` looks like a quecto configuration: a JSON object
/// carrying at least one known top-level section *shaped* as one — an
/// object, or `null` for `admission` (the pre-#2024 way to disable it).
/// A `"workflow": "build"` or `"tools": [...]` in another tool's
/// `config.json` shares a name, not a shape.
pub fn looks_like_config(document: &Value) -> bool {
    document.as_object().is_some_and(|object| {
        KNOWN_TOP_LEVEL_KEYS.iter().any(|key| {
            object.get(*key).is_some_and(|section| {
                section.is_object() || (*key == "admission" && section.is_null())
            })
        })
    })
}

/// Whether a value may stand at a global-only path in an overlay.
pub type AllowedInOverlay = fn(&Value) -> bool;

/// Settings a repository overlay may only narrow (#2136): what it may set
/// there is allowlisted, anything else is global-only. Ranking sends
/// matching code to TypeSafe, so an overlay may turn it off but never on;
/// the search log is the owner's, so an overlay may not turn it off.
pub const GLOBAL_ONLY_PATHS: &[(&str, AllowedInOverlay)] = &[
    ("tools.grep.relevance", ranking_off),
    ("tools.grep.log", logging_on),
    // #2150: the event log is the owner's too: an overlay may switch it on,
    // never off.
    ("telemetry.event_log", logging_on),
];

/// `{"enabled": false}` (ranking switched off), or `{}` (no effect: what
/// unsetting `enabled` leaves).
fn ranking_off(value: &Value) -> bool {
    *value == serde_json::json!({"enabled": false}) || *value == serde_json::json!({})
}

/// `{"enabled": true}` (the log kept or switched on), or `{}` (no effect).
fn logging_on(value: &Value) -> bool {
    *value == serde_json::json!({"enabled": true}) || *value == serde_json::json!({})
}

/// The first global-only key, or global-only path set to a value an overlay
/// may not carry, in an overlay document.
pub fn global_only_key(document: &Map<String, Value>) -> Option<&'static str> {
    GLOBAL_ONLY_KEYS
        .iter()
        .copied()
        .find(|key| document.contains_key(*key))
        .or_else(|| {
            GLOBAL_ONLY_PATHS
                .iter()
                .find(|(path, allowed)| {
                    path_in(document, path).is_some_and(|value| !allowed(value))
                })
                .map(|(path, _)| *path)
        })
}

/// Each global-only path an overlay document sets to a value an overlay may
/// not carry, with that value.
pub fn global_only_values(document: &Map<String, Value>) -> Vec<(&'static str, Value)> {
    GLOBAL_ONLY_PATHS
        .iter()
        .filter_map(|(path, allowed)| {
            path_in(document, path)
                .filter(|value| !allowed(value))
                .map(|value| (*path, value.clone()))
        })
        .collect()
}

/// The value at a dotted path of `document`, walking its objects.
fn path_in<'a>(document: &'a Map<String, Value>, path: &str) -> Option<&'a Value> {
    let mut segments = path.split('.');
    let first = document.get(segments.next()?)?;
    segments.try_fold(first, |value, segment| value.get(segment))
}

/// A top-level global-only key a dotted key path writes under, if any: an
/// overlay may not write there at all. (Global-only paths are judged by the
/// value written, after the write: see [`global_only_key`].)
pub fn global_only_path(segments: &[&str]) -> Option<&'static str> {
    GLOBAL_ONLY_KEYS
        .iter()
        .copied()
        .find(|key| segments.first() == Some(key))
}

/// Merge `overlay` over `global` by the section rules above. Both must be
/// objects; the caller has checked.
pub fn merge_overlay(
    mut global: Map<String, Value>,
    overlay: Map<String, Value>,
) -> Map<String, Value> {
    for (key, value) in overlay {
        match key.as_str() {
            "agents" => merge_depth(global.entry(key).or_insert(Value::Null), value, 2),
            "tools" => merge_tools(global.entry(key).or_insert(Value::Null), value),
            "container_configs" => {
                merge_container_configs(global.entry(key).or_insert(Value::Null), value)
            }
            "workflow" => merge_depth(global.entry(key).or_insert(Value::Null), value, 1),
            _ => {
                global.insert(key, value);
            }
        }
    }
    global
}

/// Merge objects key-wise down to `depth` levels; below that (or when
/// either side is not an object) the overlay value replaces the base.
fn merge_depth(base: &mut Value, overlay: Value, depth: usize) {
    match (base.as_object_mut(), overlay) {
        (Some(base_map), Value::Object(overlay_map)) if depth > 0 => {
            for (key, value) in overlay_map {
                merge_depth(base_map.entry(key).or_insert(Value::Null), value, depth - 1);
            }
        }
        (_, overlay) => *base = overlay,
    }
}

fn merge_tools(base: &mut Value, overlay: Value) {
    match (base.as_object_mut(), overlay) {
        (Some(base_map), Value::Object(overlay_map)) => {
            for (key, value) in overlay_map {
                let slot = base_map.entry(key.as_str()).or_insert(Value::Null);
                match key.as_str() {
                    "web" | "grep" => merge_depth(slot, value, 2),
                    "policy" => merge_depth(slot, value, 2),
                    _ => *slot = value,
                }
            }
        }
        (_, overlay) => *base = overlay,
    }
}

/// Entry-wise, with the repo-local rule: a local default un-defaults the
/// global entries so exactly one default survives the merge.
fn merge_container_configs(base: &mut Value, overlay: Value) {
    match (base.as_object_mut(), overlay) {
        (Some(base_map), Value::Object(overlay_map)) => {
            let local_default = overlay_map
                .values()
                .any(|entry| entry.get("default").and_then(Value::as_bool) == Some(true));
            if local_default {
                for entry in base_map.values_mut() {
                    if let Some(default) = entry.get_mut("default")
                        && default.as_bool() == Some(true)
                    {
                        *default = Value::Bool(false);
                    }
                }
            }
            base_map.extend(overlay_map);
        }
        (_, overlay) => *base = overlay,
    }
}

/// Split a dotted key path into its segments; empty paths and empty
/// segments are rejected.
pub fn key_segments(key_path: &str) -> Option<Vec<&str>> {
    let segments: Vec<&str> = key_path.split('.').collect();
    (!key_path.is_empty() && segments.iter().all(|segment| !segment.is_empty())).then_some(segments)
}

/// The value at `key_path`, if every segment resolves through objects.
pub fn get_path<'a>(document: &'a Value, key_path: &str) -> Option<&'a Value> {
    let segments = key_segments(key_path)?;
    segments
        .iter()
        .try_fold(document, |current, segment| current.get(segment))
}

/// Set `key_path` to `value`, creating missing intermediate objects.
/// Fails naming the segment at which a non-object stands in the way.
pub fn set_path(document: &mut Value, key_path: &str, value: Value) -> Result<(), String> {
    let segments = key_segments(key_path).ok_or_else(|| key_path.to_string())?;
    let (last, parents) = segments.split_last().expect("at least one segment");
    let mut current = document;
    let mut walked = Vec::new();
    for segment in parents {
        walked.push(*segment);
        let map = current
            .as_object_mut()
            .ok_or_else(|| walked[..walked.len() - 1].join("."))?;
        current = map
            .entry(*segment)
            .or_insert_with(|| Value::Object(Map::new()));
    }
    let map = current.as_object_mut().ok_or_else(|| walked.join("."))?;
    map.insert((*last).to_string(), value);
    Ok(())
}

/// Remove `key_path` from the document, returning the value that stood
/// there. `Ok(None)` when nothing is set at the path (a missing
/// intermediate object counts as not set); `Err` names the segment at
/// which a non-object stands in the way. Emptied parents are kept.
pub fn remove_path(document: &mut Value, key_path: &str) -> Result<Option<Value>, String> {
    let segments = key_segments(key_path).ok_or_else(|| key_path.to_string())?;
    let (last, parents) = segments.split_last().expect("at least one segment");
    let mut current = document;
    let mut walked = Vec::new();
    for segment in parents {
        let map = match current.as_object_mut() {
            Some(map) => map,
            None => return Err(walked.join(".")),
        };
        walked.push(*segment);
        match map.get_mut(*segment) {
            Some(next) => current = next,
            None => return Ok(None),
        }
    }
    match current.as_object_mut() {
        Some(map) => Ok(map.remove(*last)),
        None => Err(walked.join(".")),
    }
}

#[cfg(test)]
#[path = "overlay_policy_tests.rs"]
mod tests;
