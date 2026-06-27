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
