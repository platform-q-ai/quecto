use super::*;

fn round_trip<T: serde::Serialize + serde::de::DeserializeOwned>(v: &T) -> serde_json::Value {
    let s = serde_json::to_string(v).unwrap();
    serde_json::from_str(&s).unwrap()
}

#[test]
fn get_extensions_command_serializes() {
    let cmd = AgentCommand::GetExtensions {
        id: Some("ge-1".into()),
    };
    let j = round_trip(&cmd);
    assert_eq!(j["type"], "get_extensions");
    assert_eq!(j["id"], "ge-1");
}

#[test]
fn reload_extensions_command_serializes() {
    let cmd = AgentCommand::ReloadExtensions {
        id: Some("re-1".into()),
    };
    let j = round_trip(&cmd);
    assert_eq!(j["type"], "reload_extensions");
    assert_eq!(j["id"], "re-1");
}

#[test]
fn extensions_changed_event_matches_spec_shape() {
    let ev = AgentEvent::ExtensionsChanged {
        extensions: vec![ExtensionInfo {
            name: "greet".into(),
            description: "Say hello".into(),
            source: None,
            owner: None,
            availability: None,
        }],
    };
    let j = round_trip(&ev);
    assert_eq!(j["type"], "extensions_changed");
    assert!(j["extensions"].is_array());
    assert_eq!(j["extensions"][0]["name"], "greet");
    assert_eq!(j["extensions"][0]["description"], "Say hello");
}

#[test]
fn extensions_changed_roundtrip() {
    let ev = AgentEvent::ExtensionsChanged {
        extensions: vec![
            ExtensionInfo {
                name: "a".into(),
                description: "desc a".into(),
                source: None,
                owner: None,
                availability: None,
            },
            ExtensionInfo {
                name: "b".into(),
                description: "desc b".into(),
                source: None,
                owner: None,
                availability: None,
            },
        ],
    };
    let json = ev.to_json_line();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["extensions"].as_array().unwrap().len(), 2);
}

#[test]
fn test_parse_get_extensions_command() {
    let json = r#"{"type":"get_extensions"}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.type_name(), "get_extensions");
    assert!(cmd.id().is_none());
}

#[test]
fn test_parse_get_extensions_with_id() {
    let json = r#"{"type":"get_extensions","id":"ge-1"}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.id(), Some("ge-1"));
    assert_eq!(cmd.type_name(), "get_extensions");
}

#[test]
fn test_parse_reload_extensions_command() {
    let json = r#"{"type":"reload_extensions"}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.type_name(), "reload_extensions");
    assert!(cmd.id().is_none());
}

#[test]
fn test_parse_reload_extensions_with_id() {
    let json = r#"{"type":"reload_extensions","id":"re-1"}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.id(), Some("re-1"));
    assert_eq!(cmd.type_name(), "reload_extensions");
}

#[test]
fn test_extensions_changed_event_serializes() {
    let ev = AgentEvent::ExtensionsChanged {
        extensions: vec![
            ExtensionInfo {
                name: "greet".to_string(),
                description: "Greet the user".to_string(),
                source: None,
                owner: None,
                availability: None,
            },
            ExtensionInfo {
                name: "weather".to_string(),
                description: "Get weather".to_string(),
                source: None,
                owner: None,
                availability: None,
            },
        ],
    };
    let json = ev.to_json_line();
    assert!(json.contains("\"type\":\"extensions_changed\""));
    assert!(json.contains("\"greet\""));
    assert!(json.contains("\"weather\""));
}

#[test]
fn test_extensions_changed_event_empty_list() {
    let ev = AgentEvent::ExtensionsChanged { extensions: vec![] };
    let json = ev.to_json_line();
    assert!(json.contains("\"extensions\":[]"));
}

#[test]
fn extension_command_type_names() {
    assert_eq!(
        AgentCommand::GetExtensions { id: None }.type_name(),
        "get_extensions"
    );
    assert_eq!(
        AgentCommand::ReloadExtensions { id: None }.type_name(),
        "reload_extensions"
    );
    assert_eq!(
        AgentCommand::RegisterTools {
            id: None,
            tools: vec![]
        }
        .type_name(),
        "register_tools"
    );
    assert_eq!(
        AgentCommand::UnregisterTools {
            id: None,
            tools: vec![]
        }
        .type_name(),
        "unregister_tools"
    );
    assert_eq!(
        AgentCommand::ToolResult {
            tool_call_id: "c".into(),
            content: "x".into(),
            is_error: false
        }
        .type_name(),
        "tool_result"
    );
}
