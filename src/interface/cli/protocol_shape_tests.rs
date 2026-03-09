// Shape tests for protocol — verifies JSON serialization matches the issue #233 spec.
// Loaded from protocol.rs via #[path = "protocol_shape_tests.rs"].
#![allow(unused_imports)]
use super::*;

use super::*;

fn round_trip<T: serde::Serialize + serde::de::DeserializeOwned>(v: &T) -> serde_json::Value {
    let s = serde_json::to_string(v).unwrap();
    serde_json::from_str(&s).unwrap()
}

#[test]
fn tool_execution_start_matches_spec_shape() {
    let ev = AgentEvent::ToolExecutionStart {
        tool_call_id: "call_abc123".into(),
        tool_name: "bash".into(),
        args: serde_json::json!({"command": "cargo test"}),
    };
    let j = round_trip(&ev);
    assert_eq!(j["type"], "tool_execution_start");
    assert_eq!(j["toolCallId"], "call_abc123"); // camelCase
    assert_eq!(j["toolName"], "bash"); // camelCase
    assert!(j["args"].is_object());
    // Spec does NOT have snake_case variants
    assert!(j.get("tool_call_id").is_none());
    assert!(j.get("tool_name").is_none());
}

#[test]
fn tool_execution_end_matches_spec_shape() {
    let ev = AgentEvent::ToolExecutionEnd {
        tool_call_id: "call_abc123".into(),
        tool_name: "bash".into(),
        result: ToolResultContent {
            content: vec![serde_json::json!({"type":"text","text":"ok"})],
        },
        is_error: false,
    };
    let j = round_trip(&ev);
    assert_eq!(j["type"], "tool_execution_end");
    assert_eq!(j["toolCallId"], "call_abc123");
    assert_eq!(j["toolName"], "bash");
    assert!(j["result"]["content"].is_array());
    assert_eq!(j["isError"], false); // camelCase
    assert!(j.get("is_error").is_none());
}

#[test]
fn turn_end_matches_spec_shape() {
    let ev = AgentEvent::TurnEnd {
        message: TurnMessage {
            role: "assistant".into(),
            content: "I'll fix the failing tests...".into(),
            usage: Some(TurnUsage {
                input: 1500,
                output: 200,
                total: 1700,
            }),
            stop_reason: Some("toolUse".into()),
        },
        tool_results: vec![],
    };
    let j = round_trip(&ev);
    assert_eq!(j["type"], "turn_end");
    assert_eq!(j["message"]["role"], "assistant");
    assert_eq!(j["message"]["content"], "I'll fix the failing tests...");
    assert_eq!(j["message"]["usage"]["input"], 1500);
    assert_eq!(j["message"]["usage"]["output"], 200);
    assert_eq!(j["message"]["usage"]["total"], 1700);
    assert_eq!(j["message"]["stopReason"], "toolUse"); // camelCase
    assert!(j.get("stop_reason").is_none());
    assert!(j["toolResults"].is_array()); // camelCase
}

#[test]
fn response_with_id_matches_spec() {
    let ev = AgentEvent::ok(Some("req-1"), "prompt", None);
    let j = round_trip(&ev);
    assert_eq!(j["type"], "response");
    assert_eq!(j["id"], "req-1");
    assert_eq!(j["command"], "prompt");
    assert_eq!(j["success"], true);
    // data and error absent when None
    assert!(j.get("data").is_none());
    assert!(j.get("error").is_none());
}

#[test]
fn response_error_matches_spec() {
    let ev = AgentEvent::Response {
        id: None,
        command: "set_model".into(),
        success: false,
        data: None,
        error: Some("Model not found".into()),
    };
    let j = round_trip(&ev);
    assert_eq!(j["type"], "response");
    assert_eq!(j["command"], "set_model");
    assert_eq!(j["success"], false);
    assert_eq!(j["error"], "Model not found");
    assert!(j.get("id").is_none()); // absent when None
}

#[test]
fn get_state_data_matches_spec_shape() {
    let state = SessionState {
        model: "claude-sonnet-4-20250514".into(),
        is_streaming: false,
        session_key: "cli:my-session".into(),
        message_count: 12,
        pending_message_count: 0,
    };
    let j = round_trip(&state);
    assert_eq!(j["model"], "claude-sonnet-4-20250514");
    assert_eq!(j["isStreaming"], false); // camelCase
    assert_eq!(j["sessionKey"], "cli:my-session"); // camelCase
    assert_eq!(j["messageCount"], 12); // camelCase
    assert_eq!(j["pendingMessageCount"], 0); // camelCase
    assert!(j.get("is_streaming").is_none());
}

fn make_test_stats() -> SessionStats {
    SessionStats {
        session_key: "cli:my-session".into(),
        user_messages: 5,
        assistant_messages: 5,
        tool_calls: 12,
        tool_results: 12,
        total_messages: 22,
        tokens: TokenStats {
            input: 50000,
            output: 10000,
            cache_read: 40000,
            cache_write: 5000,
            total: 105000,
        },
        cost: 0.45,
    }
}

#[test]
fn get_session_stats_message_counts_camel_case() {
    let j = round_trip(&make_test_stats());
    assert_eq!(j["sessionKey"], "cli:my-session");
    assert_eq!(j["userMessages"], 5);
    assert_eq!(j["assistantMessages"], 5);
    assert_eq!(j["toolCalls"], 12);
    assert_eq!(j["toolResults"], 12);
    assert_eq!(j["totalMessages"], 22);
}

#[test]
fn get_session_stats_tokens_camel_case() {
    let j = round_trip(&make_test_stats());
    assert_eq!(j["tokens"]["input"], 50000);
    assert_eq!(j["tokens"]["output"], 10000);
    assert_eq!(j["tokens"]["cacheRead"], 40000);
    assert_eq!(j["tokens"]["cacheWrite"], 5000);
    assert_eq!(j["tokens"]["total"], 105000);
    assert!((j["cost"].as_f64().unwrap() - 0.45).abs() < 1e-9);
}

#[test]
fn streaming_behavior_serializes_as_camel_case() {
    // Spec: "steer" and "followUp"
    let cmd = AgentCommand::Prompt {
        id: None,
        message: "hi".into(),
        streaming_behavior: Some(StreamingBehavior::FollowUp),
    };
    let j = round_trip(&cmd);
    assert_eq!(j["streamingBehavior"], "followUp"); // camelCase value
    assert_ne!(j["streamingBehavior"], "follow_up"); // NOT snake_case
}

#[test]
fn steer_streaming_behavior_value() {
    let cmd = AgentCommand::Prompt {
        id: None,
        message: "hi".into(),
        streaming_behavior: Some(StreamingBehavior::Steer),
    };
    let j = round_trip(&cmd);
    assert_eq!(j["streamingBehavior"], "steer");
}

#[test]
fn agent_end_has_messages_array() {
    let ev = AgentEvent::AgentEnd {
        messages: vec![serde_json::json!({"role":"assistant","content":"ok"})],
    };
    let j = round_trip(&ev);
    assert_eq!(j["type"], "agent_end");
    assert!(j["messages"].is_array());
    assert_eq!(j["messages"].as_array().unwrap().len(), 1);
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
            },
            ExtensionInfo {
                name: "b".into(),
                description: "desc b".into(),
            },
        ],
    };
    let json = ev.to_json_line();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["extensions"].as_array().unwrap().len(), 2);
}

#[test]
fn follow_up_command_serializes_type_as_follow_up() {
    // The spec uses "follow_up" (snake_case) for the type field
    let cmd = AgentCommand::FollowUp {
        id: None,
        message: "also do this".into(),
    };
    let j = round_trip(&cmd);
    assert_eq!(j["type"], "follow_up");
}

#[test]
fn roundtrip_parse_streaming_behavior_follow_up() {
    // Spec sends "followUp" camelCase; we must parse it back
    let raw = r#"{"type":"prompt","message":"hi","streamingBehavior":"followUp"}"#;
    let cmd: AgentCommand = serde_json::from_str(raw).unwrap();
    match cmd {
        AgentCommand::Prompt {
            streaming_behavior: Some(StreamingBehavior::FollowUp),
            ..
        } => {}
        _ => panic!("expected FollowUp streaming behavior"),
    }
}

// ─── Extension commands (moved from protocol.rs inline tests) ─────────────

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
            },
            ExtensionInfo {
                name: "weather".to_string(),
                description: "Get weather".to_string(),
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

// ─── UDS extension protocol commands (#352) ───────────────────────────────

#[test]
fn register_tools_command_parses() {
    let json = r#"{"type":"register_tools","id":"rt-1","tools":[{"name":"weather","description":"Get weather"}]}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.type_name(), "register_tools");
    assert_eq!(cmd.id(), Some("rt-1"));
    match cmd {
        AgentCommand::RegisterTools { tools, .. } => {
            assert_eq!(tools.len(), 1);
            assert_eq!(tools[0].name, "weather");
            assert_eq!(tools[0].description, "Get weather");
        }
        _ => panic!("expected RegisterTools"),
    }
}

#[test]
fn register_tools_with_schema_parses() {
    let json = r#"{"type":"register_tools","tools":[{"name":"weather","description":"Get weather","parametersSchema":"{\"type\":\"object\",\"properties\":{\"city\":{\"type\":\"string\"}}}"}]}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    match cmd {
        AgentCommand::RegisterTools { tools, .. } => {
            assert!(tools[0].parameters_schema.contains("city"));
        }
        _ => panic!("expected RegisterTools"),
    }
}

#[test]
fn register_tools_default_schema() {
    let json = r#"{"type":"register_tools","tools":[{"name":"test","description":"Test"}]}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    match cmd {
        AgentCommand::RegisterTools { tools, .. } => {
            assert_eq!(tools[0].parameters_schema, r#"{"type":"object"}"#);
        }
        _ => panic!("expected RegisterTools"),
    }
}

#[test]
fn unregister_tools_command_parses() {
    let json = r#"{"type":"unregister_tools","id":"ut-1","tools":["weather"]}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.type_name(), "unregister_tools");
    match cmd {
        AgentCommand::UnregisterTools { tools, .. } => {
            assert_eq!(tools, vec!["weather"]);
        }
        _ => panic!("expected UnregisterTools"),
    }
}

#[test]
fn tool_result_command_parses() {
    let json = r#"{"type":"tool_result","toolCallId":"call-1","content":"22°C","isError":false}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.type_name(), "tool_result");
    assert!(cmd.id().is_none());
    match cmd {
        AgentCommand::ToolResult {
            tool_call_id,
            content,
            is_error,
        } => {
            assert_eq!(tool_call_id, "call-1");
            assert_eq!(content, "22°C");
            assert!(!is_error);
        }
        _ => panic!("expected ToolResult"),
    }
}

#[test]
fn tool_result_error_parses() {
    let json =
        r#"{"type":"tool_result","toolCallId":"call-2","content":"not found","isError":true}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    match cmd {
        AgentCommand::ToolResult { is_error, .. } => assert!(is_error),
        _ => panic!("expected ToolResult"),
    }
}

#[test]
fn execute_tool_event_serializes() {
    let ev = AgentEvent::ExecuteTool {
        tool_call_id: "call-1".into(),
        tool_name: "weather".into(),
        arguments: r#"{"city":"London"}"#.into(),
    };
    let j = round_trip(&ev);
    assert_eq!(j["type"], "execute_tool");
    assert_eq!(j["toolCallId"], "call-1");
    assert_eq!(j["toolName"], "weather");
    assert_eq!(j["arguments"], r#"{"city":"London"}"#);
}

#[test]
fn execute_tool_event_roundtrip() {
    let ev = AgentEvent::ExecuteTool {
        tool_call_id: "c1".into(),
        tool_name: "test".into(),
        arguments: "{}".into(),
    };
    let json = ev.to_json_line();
    let parsed: AgentEvent = serde_json::from_str(&json).unwrap();
    match parsed {
        AgentEvent::ExecuteTool {
            tool_call_id,
            tool_name,
            arguments,
        } => {
            assert_eq!(tool_call_id, "c1");
            assert_eq!(tool_name, "test");
            assert_eq!(arguments, "{}");
        }
        _ => panic!("expected ExecuteTool"),
    }
}
