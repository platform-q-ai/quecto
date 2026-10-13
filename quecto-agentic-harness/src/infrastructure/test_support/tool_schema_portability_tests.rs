use super::portability_violations;
use serde_json::json;

#[test]
fn a_typed_object_schema_is_portable() {
    let schema = json!({
        "type": "object",
        "properties": {
            "path": {"type": "string", "description": "d"},
            "limit": {"type": ["integer", "null"]},
            "names": {"type": "array", "items": {"type": "string"}},
            "spec": {"type": "object", "properties": {"template": {"type": "object"}}},
            "mode": {"type": "string", "enum": ["a", "b"]}
        }
    });
    assert_eq!(portability_violations(&schema), Vec::<String>::new());
}

#[test]
fn a_property_with_only_a_description_is_untyped() {
    let schema = json!({
        "type": "object",
        "properties": {"container": {"description": "Launch location"}}
    });
    assert_eq!(
        portability_violations(&schema),
        vec!["$.container: no type (and no typed anyOf)".to_string()]
    );
}

#[test]
fn an_any_of_of_typed_branches_is_portable_and_its_branches_are_checked() {
    let typed = json!({
        "type": "object",
        "properties": {"container": {"description": "d", "anyOf": [
            {"type": "boolean"},
            {"type": "object", "properties": {"mode": {"type": "string"}}}
        ]}}
    });
    assert_eq!(portability_violations(&typed), Vec::<String>::new());

    let untyped_branch_field = json!({
        "type": "object",
        "properties": {"container": {"anyOf": [
            {"type": "boolean"},
            {"type": "object", "properties": {"mode": {"description": "m"}}}
        ]}}
    });
    assert_eq!(
        portability_violations(&untyped_branch_field),
        vec!["$.container.anyOf[1].mode: no type (and no typed anyOf)".to_string()]
    );
}

#[test]
fn malformed_any_of_and_type_lists_are_violations() {
    let schema = json!({
        "type": "object",
        "properties": {
            "empty": {"anyOf": []},
            "both": {"type": "string", "anyOf": [{"type": "string"}]},
            "unknown": {"type": "text"},
            "no_names": {"type": []},
            "items": {"type": "array", "items": {"description": "i"}}
        }
    });
    let violations = portability_violations(&schema);
    for expected in [
        "$.empty: anyOf is not a non-empty list",
        "$.both: both type and anyOf; declare one",
        "$.unknown: unknown type \"text\"",
        "$.no_names: type is not a name or a list of names",
        "$.items[]: no type (and no typed anyOf)",
    ] {
        assert!(
            violations.iter().any(|v| v == expected),
            "missing {expected:?} in {violations:?}"
        );
    }
    assert_eq!(violations.len(), 5, "{violations:?}");
}

#[test]
fn keywords_outside_the_allowlists_are_violations() {
    let schema = json!({
        "type": "object",
        "anyOf": [{"type": "object"}],
        "properties": {
            "x": {"type": "string", "oneOf": [{"type": "string"}]},
            "r": {"$ref": "#/defs/r"},
            "f": {"type": "string", "format": "uri"}
        }
    });
    assert_eq!(
        portability_violations(&schema),
        vec![
            "$: keyword anyOf is outside the portable subset".to_string(),
            "$.x: keyword oneOf is outside the portable subset".to_string(),
            "$.r: keyword $ref is outside the portable subset".to_string(),
            "$.r: no type (and no typed anyOf)".to_string(),
            "$.f: keyword format is outside the portable subset".to_string(),
        ]
    );
    // The root admits fewer keywords than a property does.
    assert_eq!(
        portability_violations(&json!({"type": "object", "enum": [{}], "minimum": 1})),
        vec![
            "$: keyword enum is outside the portable subset".to_string(),
            "$: keyword minimum is outside the portable subset".to_string(),
        ]
    );
}

#[test]
fn arrays_need_items_and_items_need_an_array() {
    let schema = json!({
        "type": "object",
        "properties": {
            "bare": {"type": "array"},
            "nullable": {"type": ["array", "null"]},
            "stray": {"type": "string", "items": {"type": "string"}},
            "fine": {"type": ["array", "null"], "items": {"type": "integer"}}
        }
    });
    assert_eq!(
        portability_violations(&schema),
        vec![
            "$.bare: an array type without items".to_string(),
            "$.nullable: an array type without items".to_string(),
            "$.stray: items without an array type".to_string(),
        ]
    );
}

#[test]
fn required_and_additional_properties_are_checked_at_every_level() {
    let schema = json!({
        "type": "object",
        "required": ["a", "missing"],
        "additionalProperties": {"type": "string"},
        "properties": {
            "a": {
                "type": "object",
                "required": "b",
                "additionalProperties": false,
                "properties": {"b": {"type": "string"}}
            },
            "c": {"type": "object", "required": [1], "additionalProperties": "no"}
        }
    });
    assert_eq!(
        portability_violations(&schema),
        vec![
            "$.a: required is not a list".to_string(),
            "$.c: required names 1, not a property".to_string(),
            "$.c: additionalProperties is not a boolean".to_string(),
            "$: required names \"missing\", not a property".to_string(),
            "$: additionalProperties is not a boolean".to_string(),
        ]
    );
}

#[test]
fn root_shapes_are_violations() {
    assert_eq!(
        portability_violations(&json!({"type": "array"})),
        vec!["$: the root type is not \"object\"".to_string()]
    );
    assert_eq!(
        portability_violations(&json!({"properties": {}})),
        vec!["$: the root type is not \"object\"".to_string()]
    );
    assert_eq!(
        portability_violations(&json!("schema")),
        vec!["$: the schema is not a JSON object".to_string()]
    );
    assert_eq!(
        portability_violations(&json!({"type": "object", "properties": []})),
        vec!["$: properties is not an object".to_string()]
    );
}

#[test]
fn an_any_of_node_carries_only_its_branches_and_a_description() {
    let schema = json!({
        "type": "object",
        "properties": {
            "c": {
                "description": "d",
                "anyOf": [{"type": "boolean"}],
                "required": ["x"],
                "items": {}
            }
        }
    });
    assert_eq!(
        portability_violations(&schema),
        vec![
            "$.c: keyword required is outside the portable subset".to_string(),
            "$.c: keyword items is outside the portable subset".to_string(),
        ]
    );
}

#[test]
fn object_keywords_need_an_object_type() {
    let schema = json!({
        "type": "object",
        "properties": {
            "s": {
                "type": "string",
                "properties": {"x": {"type": "string"}},
                "required": ["x"],
                "additionalProperties": false
            },
            "o": {"type": ["object", "null"], "properties": {"x": {"type": "string"}}}
        }
    });
    assert_eq!(
        portability_violations(&schema),
        vec![
            "$.s: properties without an object type".to_string(),
            "$.s: required without an object type".to_string(),
            "$.s: additionalProperties without an object type".to_string(),
        ]
    );
}

#[test]
fn annotation_and_constraint_values_have_their_shapes() {
    let schema = json!({
        "type": "object",
        "description": 1,
        "properties": {
            "e": {"type": "string", "enum": []},
            "f": {"type": "string", "enum": "a"},
            "n": {"type": "integer", "minimum": "1", "maximum": 2},
            "m": {"type": "integer", "maximum": null},
            "d": {"type": "string", "description": ["d"]},
            "a": {"anyOf": [{"type": "string"}], "description": 2}
        }
    });
    assert_eq!(
        portability_violations(&schema),
        vec![
            "$: description is not a string".to_string(),
            "$.e: enum is not a non-empty list".to_string(),
            "$.f: enum is not a non-empty list".to_string(),
            "$.n: minimum is not a number".to_string(),
            "$.m: maximum is not a number".to_string(),
            "$.d: description is not a string".to_string(),
            "$.a: description is not a string".to_string(),
        ]
    );
}
