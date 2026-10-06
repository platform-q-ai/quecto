//! The spawn schema types `container` (an untyped one failed every
//! container spawn from a model that quoted it).

use super::*;

#[test]
fn test_definition_types_container_as_boolean_or_object() {
    // An untyped `container` left models to guess, and some sent the object
    // as a quoted JSON string, failing every container spawn.
    let def = SpawnTool::new(vec![]).definition();
    let schema: serde_json::Value = serde_json::from_str(&def.parameters_schema).unwrap();
    let container = &schema["properties"]["container"];
    let branches = container["anyOf"].as_array().expect("container has anyOf");
    assert_eq!(branches.len(), 4, "{container}");
    assert_eq!(branches[0], serde_json::json!({"type": "boolean"}));
    // One object branch per accepted object shape, each admitting exactly
    // the fields the parser takes for it: `new` with optional
    // container_config/name, `existing` with exactly one of ref or name.
    for (branch, mode, fields, required) in [
        (
            &branches[1],
            "new",
            &["container_config", "mode", "name"][..],
            &["mode"][..],
        ),
        (
            &branches[2],
            "existing",
            &["mode", "ref"][..],
            &["mode", "ref"][..],
        ),
        (
            &branches[3],
            "existing",
            &["mode", "name"][..],
            &["mode", "name"][..],
        ),
    ] {
        assert_eq!(branch["type"], "object", "{branch}");
        assert_eq!(branch["required"], serde_json::json!(required), "{branch}");
        assert_eq!(branch["additionalProperties"], false, "{branch}");
        assert_eq!(
            branch["properties"]["mode"],
            serde_json::json!({"type": "string", "enum": [mode]})
        );
        let mut names: Vec<&str> = branch["properties"]
            .as_object()
            .expect("object properties")
            .keys()
            .map(String::as_str)
            .collect();
        names.sort_unstable();
        assert_eq!(names, fields, "{branch}");
        for field in fields.iter().filter(|field| **field != "mode") {
            assert_eq!(branch["properties"][field]["type"], "string", "{branch}");
        }
    }
    assert!(
        container["description"]
            .as_str()
            .is_some_and(|d| d.contains("never a quoted JSON string")),
        "description kept and tells models not to quote"
    );
}

#[test]
fn every_object_the_schema_describes_in_full_is_one_the_parser_accepts() {
    let def = SpawnTool::new(vec![]).definition();
    let schema: serde_json::Value = serde_json::from_str(&def.parameters_schema).unwrap();
    let branches = schema["properties"]["container"]["anyOf"]
        .as_array()
        .expect("container has anyOf");
    let objects: Vec<&serde_json::Value> = branches
        .iter()
        .filter(|branch| branch["type"] == "object")
        .collect();
    assert_eq!(objects.len(), 3);
    for branch in objects {
        let properties = branch["properties"].as_object().expect("properties");
        // Every property set (the fullest shape), then only the required.
        let full: serde_json::Map<String, serde_json::Value> = properties
            .iter()
            .map(|(name, property)| {
                let value = property["enum"][0].clone();
                let value = if value.is_null() { "x".into() } else { value };
                (name.clone(), value)
            })
            .collect();
        let required: serde_json::Map<String, serde_json::Value> = full
            .iter()
            .filter(|(name, _)| {
                branch["required"]
                    .as_array()
                    .expect("required")
                    .iter()
                    .any(|r| r == name.as_str())
            })
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        for container in [full, required] {
            let args = serde_json::json!({ "container": container });
            assert!(
                super::super::spawn_input::parse_container_selection(&args).is_ok(),
                "schema-valid {args} refused"
            );
        }
    }
    for flag in [true, false] {
        let args = serde_json::json!({ "container": flag });
        assert!(super::super::spawn_input::parse_container_selection(&args).is_ok());
    }
}

/// Every non-OpenAI parent in two weeks of audit logs quoted `container`
/// (fireworks glm-5p3-flash 35/35, glm-5p3 7/7, anthropic-api
/// claude-opus-5-5 19/19). The tool reads arguments the same way whatever
/// provider produced them, so the glm-5p3-flash shape launches too.
#[test]
fn a_quoted_container_from_any_provider_parses_at_the_tool_boundary() {
    let tool = SpawnTool::new(vec![]);
    for arguments in [
        // fireworks/accounts/fireworks/models/glm-5p3-flash
        r#"{"agent_id":"spike-1839","task":"t","container":"{\"mode\": \"new\", \"container_config\": \"quecto\", \"name\": \"spike-1839\"}"}"#,
        // anthropic-api/claude-opus-5-5
        r#"{"agent_id":"l1-red-2359","task":"t","container":"{\"mode\":\"new\",\"container_config\":\"quecto\",\"name\":\"l1-red-2359\"}"}"#,
    ] {
        let config = tool.parse_args(arguments).expect(arguments);
        assert!(
            matches!(
                &config.container,
                crate::domain::subagent::ContainerSelection::New {
                    container_config: Some(config_name),
                    name: Some(_),
                } if config_name == "quecto"
            ),
            "{arguments}: {:?}",
            config.container
        );
    }
}

/// #2461 review: the parent opts a child into the coordinator role with a
/// boolean the schema declares.
#[test]
fn the_schema_declares_the_coordinator_opt_in_as_a_boolean() {
    let def = SpawnTool::new(vec![]).definition();
    let schema: serde_json::Value = serde_json::from_str(&def.parameters_schema).unwrap();
    assert_eq!(schema["properties"]["coordinator"]["type"], "boolean");
}
