//! The tool-schema subset every provider adapter can send unchanged.
//!
//! The Anthropic (API key and Claude Code OAuth), OpenAI chat-completions
//! (OpenAI, xAI and OpenAI-compatible endpoints) and OpenAI Responses
//! (API key and ChatGPT Codex, `strict: false`) adapters all pass a tool's
//! `parameters_schema` through verbatim. A schema stays inside what each of
//! them documents when:
//!
//! - every node carries only allowlisted keywords: the root `type`
//!   (`"object"`), `properties`, `required` (names of those properties),
//!   `additionalProperties` (a boolean) and `description` (OpenAI refuses a
//!   combinator at the root of function parameters); a property, array
//!   item or `anyOf` branch those plus `items`, `enum`, `minimum` and
//!   `maximum`, or only `anyOf` and `description`; `properties`,
//!   `required` and `additionalProperties` only on an object type;
//!   `description` a string, `enum` a non-empty list, bounds numbers;
//! - every property, array item and `anyOf` branch names its JSON type,
//!   either as one allowlisted type name or a list of them, or is an
//!   `anyOf` whose every branch does. A property with only a description
//!   leaves the model to guess the type, and some models then send a
//!   quoted JSON string (spawn's untyped `container`: every container spawn
//!   failed with "container must be false, true, or an object");
//! - an array type carries `items` (OpenAI refuses an array without it);
//! - the only combinator is `anyOf` (OpenAI's documented subset has no
//!   `oneOf`, `allOf` or `not`).

use serde_json::{Map, Value};

/// The JSON Schema type names every adapter's provider accepts.
const TYPE_NAMES: &[&str] = &[
    "string", "number", "integer", "boolean", "object", "array", "null",
];

/// The only keywords the root may carry.
const ROOT_KEYWORDS: &[&str] = &[
    "type",
    "properties",
    "required",
    "additionalProperties",
    "description",
];

/// The only keywords a property, array item or `anyOf` branch may carry.
const NODE_KEYWORDS: &[&str] = &[
    "type",
    "description",
    "properties",
    "required",
    "additionalProperties",
    "items",
    "enum",
    "anyOf",
    "minimum",
    "maximum",
];

/// The keywords only an object type may carry.
const OBJECT_KEYWORDS: &[&str] = &["properties", "required", "additionalProperties"];

/// The only keywords an `anyOf` node may carry beside its branches.
const ANY_OF_KEYWORDS: &[&str] = &["anyOf", "description"];

/// Every way `schema` leaves the portable subset, as `path: reason` lines;
/// empty when it is portable.
pub fn portability_violations(schema: &Value) -> Vec<String> {
    let mut violations = Vec::new();
    let Some(root) = schema.as_object() else {
        violations.push("$: the schema is not a JSON object".to_string());
        return violations;
    };
    check_keywords(root, ROOT_KEYWORDS, "$", &mut violations);
    check_value_shapes(root, "$", &mut violations);
    if root.get("type").and_then(Value::as_str) != Some("object") {
        violations.push("$: the root type is not \"object\"".to_string());
    }
    check_object_keywords(root, "$", &mut violations);
    violations
}

/// Every keyword of `map` is one `allowed` names.
fn check_keywords(
    map: &Map<String, Value>,
    allowed: &[&str],
    path: &str,
    violations: &mut Vec<String>,
) {
    for keyword in map.keys() {
        if !allowed.contains(&keyword.as_str()) {
            violations.push(format!(
                "{path}: keyword {keyword} is outside the portable subset"
            ));
        }
    }
}

/// `properties` (each typed), `required` (names of those properties) and
/// `additionalProperties` (a boolean) of an object node.
fn check_object_keywords(map: &Map<String, Value>, path: &str, violations: &mut Vec<String>) {
    let mut property_names: Vec<&str> = Vec::new();
    if let Some(properties) = map.get("properties") {
        match properties.as_object() {
            Some(properties) => {
                for (name, property) in properties {
                    property_names.push(name);
                    check_typed(property, &format!("{path}.{name}"), violations);
                }
            }
            None => violations.push(format!("{path}: properties is not an object")),
        }
    }
    if let Some(required) = map.get("required") {
        check_required(required, &property_names, path, violations);
    }
    if let Some(additional) = map.get("additionalProperties") {
        if !additional.is_boolean() {
            violations.push(format!("{path}: additionalProperties is not a boolean"));
        }
    }
}

fn check_required(
    required: &Value,
    property_names: &[&str],
    path: &str,
    violations: &mut Vec<String>,
) {
    let Some(names) = required.as_array() else {
        violations.push(format!("{path}: required is not a list"));
        return;
    };
    for name in names {
        let declared = name
            .as_str()
            .is_some_and(|name| property_names.contains(&name));
        if !declared {
            violations.push(format!("{path}: required names {name}, not a property"));
        }
    }
}

/// A property, item or `anyOf` branch must name its type.
fn check_typed(node: &Value, path: &str, violations: &mut Vec<String>) {
    let Some(map) = node.as_object() else {
        violations.push(format!("{path}: the schema is not a JSON object"));
        return;
    };
    check_value_shapes(map, path, violations);
    match (map.get("type"), map.get("anyOf")) {
        (Some(declared), None) => {
            check_keywords(map, NODE_KEYWORDS, path, violations);
            let names = check_type_names(declared, path, violations);
            match (names.contains(&"array"), map.get("items")) {
                (true, Some(items)) => check_typed(items, &format!("{path}[]"), violations),
                (true, None) => violations.push(format!("{path}: an array type without items")),
                (false, Some(_)) => {
                    violations.push(format!("{path}: items without an array type"));
                }
                (false, None) => {}
            }
            if names.contains(&"object") {
                check_object_keywords(map, path, violations);
            } else {
                for keyword in OBJECT_KEYWORDS {
                    if map.contains_key(*keyword) {
                        violations.push(format!("{path}: {keyword} without an object type"));
                    }
                }
            }
        }
        (None, Some(branches)) => {
            check_keywords(map, ANY_OF_KEYWORDS, path, violations);
            check_branches(branches, path, violations);
        }
        (Some(_), Some(_)) => {
            check_keywords(map, NODE_KEYWORDS, path, violations);
            violations.push(format!("{path}: both type and anyOf; declare one"));
        }
        (None, None) => {
            check_keywords(map, NODE_KEYWORDS, path, violations);
            violations.push(format!("{path}: no type (and no typed anyOf)"));
        }
    }
}

/// The values of the annotation and constraint keywords have their JSON
/// Schema shapes.
fn check_value_shapes(map: &Map<String, Value>, path: &str, violations: &mut Vec<String>) {
    if map.get("description").is_some_and(|d| !d.is_string()) {
        violations.push(format!("{path}: description is not a string"));
    }
    if let Some(values) = map.get("enum") {
        let listed = values.as_array().is_some_and(|values| !values.is_empty());
        if !listed {
            violations.push(format!("{path}: enum is not a non-empty list"));
        }
    }
    for bound in ["minimum", "maximum"] {
        if map.get(bound).is_some_and(|b| !b.is_number()) {
            violations.push(format!("{path}: {bound} is not a number"));
        }
    }
}

/// The declared type names, each checked against [`TYPE_NAMES`].
fn check_type_names<'a>(
    declared: &'a Value,
    path: &str,
    violations: &mut Vec<String>,
) -> Vec<&'a str> {
    let names: Vec<&Value> = match declared {
        Value::Array(names) if !names.is_empty() => names.iter().collect(),
        Value::String(_) => vec![declared],
        _ => {
            violations.push(format!("{path}: type is not a name or a list of names"));
            return Vec::new();
        }
    };
    let mut known = Vec::new();
    for name in names {
        match name.as_str().filter(|name| TYPE_NAMES.contains(name)) {
            Some(name) => known.push(name),
            None => violations.push(format!("{path}: unknown type {name}")),
        }
    }
    known
}

fn check_branches(branches: &Value, path: &str, violations: &mut Vec<String>) {
    match branches.as_array() {
        Some(branches) if !branches.is_empty() => {
            for (index, branch) in branches.iter().enumerate() {
                check_typed(branch, &format!("{path}.anyOf[{index}]"), violations);
            }
        }
        _ => violations.push(format!("{path}: anyOf is not a non-empty list")),
    }
}

#[cfg(test)]
#[path = "tool_schema_portability_tests.rs"]
mod tests;
