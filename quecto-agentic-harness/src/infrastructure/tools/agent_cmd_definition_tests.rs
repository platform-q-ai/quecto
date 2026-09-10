use super::*;
use crate::domain::tool::Tool;

#[test]
fn definition_does_not_expose_await() {
    let def = AgentCmdTool::new(new_registry()).definition();
    assert!(!def.description.contains("await"));
    let schema: serde_json::Value = serde_json::from_str(&def.parameters_schema).unwrap();
    let commands = schema["properties"]["command"]["enum"].as_array().unwrap();
    assert!(!commands.iter().any(|v| v.as_str() == Some("await")));
    assert!(
        !commands
            .iter()
            .any(|v| v.as_str() == Some("get_tool_catalogue"))
    );
    assert!(!commands.iter().any(|v| v.as_str() == Some("list_tools")));
    assert!(!def.description.contains("get_tool_catalogue"));
    assert!(!def.description.contains("list_tools"));
    assert!(schema["properties"].get("timeout").is_none());
    assert!(schema["properties"].get("idle_timeout").is_none());
}

#[test]
fn advertised_truncation_recovery_is_callable_through_tool_schema() {
    let schema: serde_json::Value = serde_json::from_str(
        &AgentCmdTool::new(new_registry())
            .definition()
            .parameters_schema,
    )
    .unwrap();
    assert!(
        schema["properties"]["command"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "get_message")
    );
    for field in ["messageId", "toolCallId", "offset", "limit"] {
        assert!(
            schema["properties"].get(field).is_some(),
            "missing recovery input {field}"
        );
    }
}

#[test]
fn latest_report_is_a_separate_cursor_neutral_command() {
    let (_, wire, command) = super::super::agent_cmd_parse::build_command(&serde_json::json!({
        "agent_id":"11111111-1111-4111-8111-111111111111", "command":"get_report"
    }))
    .expect("latest report should not require draining raw history");
    assert_eq!(command, "get_report");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&wire).unwrap()["type"],
        "get_report"
    );
}

#[test]
fn supervisor_budget_command_preserves_explicit_limit_and_unknown_usage_policy() {
    let (_, wire, _) = super::super::agent_cmd_parse::build_command(&serde_json::json!({
        "agent_id":"11111111-1111-4111-8111-111111111111", "command":"swarm_control",
        "action":"usage_budget", "token_limit":1000, "strict_unknown":false
    }))
    .unwrap();
    let command: serde_json::Value = serde_json::from_str(&wire).unwrap();
    assert_eq!(command["token_limit"], 1000);
    assert_eq!(command["strict_unknown"], false);
}

#[test]
fn report_export_option_is_preserved_and_type_checked() {
    let (_, wire, _) = super::super::agent_cmd_parse::build_command(&serde_json::json!({"agent_id":"11111111-1111-4111-8111-111111111111", "command":"get_report", "export_raw":true})).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&wire).unwrap()["export_raw"],
        true
    );
}

/// #1729: the supervisor closes an ended run or grants it deadline time.
#[test]
fn supervisor_close_and_extend_commands_are_typed() {
    let (_, wire, _) = super::super::agent_cmd_parse::build_command(&serde_json::json!({
        "agent_id":"11111111-1111-4111-8111-111111111111", "command":"swarm_control",
        "action":"extend", "deadline_seconds":600
    }))
    .unwrap();
    let command: serde_json::Value = serde_json::from_str(&wire).unwrap();
    assert_eq!(command["action"], "extend");
    assert_eq!(command["deadline_seconds"], 600);
    let (_, wire, _) = super::super::agent_cmd_parse::build_command(&serde_json::json!({
        "agent_id":"11111111-1111-4111-8111-111111111111", "command":"swarm_control",
        "action":"close"
    }))
    .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&wire).unwrap()["action"],
        "close"
    );
    for seconds in [
        serde_json::json!(0),
        serde_json::json!(-5),
        serde_json::json!("600"),
        serde_json::Value::Null,
    ] {
        let refused = super::super::agent_cmd_parse::build_command(&serde_json::json!({
            "agent_id":"11111111-1111-4111-8111-111111111111", "command":"swarm_control",
            "action":"extend", "deadline_seconds":seconds
        }))
        .expect_err("extend needs positive seconds");
        assert!(refused.contains("deadline_seconds"), "{refused}");
    }
}
