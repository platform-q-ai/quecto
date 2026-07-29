use super::*;

#[test]
fn definition_presents_get_messages_count_not_tail() {
    let tool = AgentCmdTool::new(new_registry());
    let def = tool.definition();
    assert!(def.description.contains("get_messages"));
    assert!(def.description.contains("count"));
    assert!(!def.description.contains("get_messages_tail"));
    let schema: serde_json::Value = serde_json::from_str(&def.parameters_schema).unwrap();
    let command_enum = schema["properties"]["command"]["enum"].as_array().unwrap();
    assert!(
        command_enum
            .iter()
            .any(|v| v.as_str() == Some("get_messages"))
    );
    assert!(
        !command_enum
            .iter()
            .any(|v| v.as_str() == Some("get_messages_tail"))
    );
}

#[test]
fn definition_hides_await_when_not_visible_in_schema() {
    // Short-term: AWAIT_VISIBLE_IN_SCHEMA=false hides await from the model.
    let tool = AgentCmdTool::new(new_registry());
    let def = tool.definition();
    if AWAIT_VISIBLE_IN_SCHEMA {
        assert!(def.description.contains("await"));
    } else {
        assert!(
            !def.description.contains("await"),
            "await must stay out of the tool description while hidden"
        );
    }
}

#[test]
fn definition_schema_await_visibility_matches_flag() {
    let tool = AgentCmdTool::new(new_registry());
    let def = tool.definition();
    let schema: serde_json::Value = serde_json::from_str(&def.parameters_schema).unwrap();
    let command_enum = schema["properties"]["command"]["enum"].as_array().unwrap();
    let has_await = command_enum.iter().any(|v| v.as_str() == Some("await"));
    assert_eq!(
        has_await, AWAIT_VISIBLE_IN_SCHEMA,
        "command enum await presence must match AWAIT_VISIBLE_IN_SCHEMA"
    );
}

#[test]
fn definition_schema_timeout_params_match_await_visibility() {
    let tool = AgentCmdTool::new(new_registry());
    let def = tool.definition();
    let schema: serde_json::Value = serde_json::from_str(&def.parameters_schema).unwrap();
    if !AWAIT_VISIBLE_IN_SCHEMA {
        assert!(schema["properties"].get("timeout").is_none());
        assert!(schema["properties"].get("idle_timeout").is_none());
        return;
    }
    assert!(schema["properties"]["timeout"].is_object());
    assert!(schema["properties"]["idle_timeout"].is_object());
}
