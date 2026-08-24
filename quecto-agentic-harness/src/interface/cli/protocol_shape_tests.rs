// Shape tests for protocol — verifies JSON serialization matches the issue #233 spec.
// Loaded from protocol.rs via #[path = "protocol_shape_tests.rs"].
#![allow(unused_imports)]
use super::*;

#[path = "protocol_shape_tool_catalogue_tests.rs"]
mod tool_catalogue_shape_tests;
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
            content: String::new(),
            message_refs: vec!["aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into()],
            usage: Some(TurnUsage {
                input: 1500,
                output: 200,
                total: 1700,
            }),
            stop_reason: Some("toolUse".into()),
            context_tokens: Some(1_200),
            max_context_tokens: Some(200_000),
            content_length: None,
        },
        tool_results: vec![],
    };
    let j = round_trip(&ev);
    assert_eq!(j["type"], "turn_end");
    assert_eq!(j["message"]["role"], "assistant");
    assert_eq!(j["message"]["content"], "");
    assert_eq!(
        j["message"]["messageRefs"][0],
        "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
    );
    assert_eq!(j["message"]["usage"]["input"], 1500);
    assert_eq!(j["message"]["usage"]["output"], 200);
    assert_eq!(j["message"]["usage"]["total"], 1700);
    assert_eq!(j["message"]["stopReason"], "toolUse"); // camelCase
    assert_eq!(j["message"]["contextTokens"], 1_200); // camelCase
    assert_eq!(j["message"]["maxContextTokens"], 200_000); // camelCase
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
    let state = crate::interface::cli::uds_session::AgentSession::new(
        "claude-sonnet-4-6".into(),
        "cli:my-session".into(),
    )
    .state_snapshot(12, None, 200_000, None);
    let j = round_trip(&state);
    assert_eq!(j["model"], "claude-sonnet-4-6");
    assert_eq!(j["isStreaming"], false); // camelCase
    assert_eq!(j["sessionKey"], "cli:my-session"); // camelCase
    assert_eq!(j["messageCount"], 12); // camelCase
    assert_eq!(j["pendingMessageCount"], 0); // camelCase // camelCase
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
            total: 60000,
        },
        cost: 0.45,
        context_tokens: 12_345,
        max_context_tokens: 200_000,
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
    assert_eq!(j["tokens"]["total"], 60000);
    assert!(
        j.get("cost").is_none(),
        "get_session_stats must not report misleading monetary cost: {j}"
    );
    assert_eq!(j["maxContextTokens"], 200_000);
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
            assert_eq!(tools[0].stable_id, None);
        }
        _ => panic!("expected RegisterTools"),
    }
}

#[test]
fn register_tools_with_schema_parses() {
    let json = r#"{"type":"register_tools","tools":[{"name":"weather","description":"Get weather","parametersSchema":"{\"type\":\"object\",\"properties\":{\"city\":{\"type\":\"string\"}}}","stableId":"com.example.weather.v1"}]}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    match cmd {
        AgentCommand::RegisterTools { tools, .. } => {
            assert!(tools[0].parameters_schema.contains("city"));
            assert_eq!(
                tools[0].stable_id.as_deref(),
                Some("com.example.weather.v1")
            );
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

// ─── Subagent protocol (#524) ─────────────────────────────────────────────────

#[test]
fn get_subagents_command_serializes() {
    let cmd = AgentCommand::GetSubagents {
        id: Some("gs-1".into()),
        since: None,
    };
    let j = round_trip(&cmd);
    assert_eq!(j["type"], "get_subagents");
    assert_eq!(j["id"], "gs-1");
}

#[test]
fn get_subagents_command_parses_without_id() {
    let raw = r#"{"type":"get_subagents"}"#;
    let cmd: AgentCommand = serde_json::from_str(raw).unwrap();
    assert_eq!(cmd.type_name(), "get_subagents");
    assert!(cmd.id().is_none());
}

#[test]
fn subagent_info_camel_case_serialization() {
    let info = SubagentInfo {
        agent_uuid: None,
        display_name: None,
        agent_id: "test-agent".into(),
        status: "running".into(),
        liveness: None,
        last_tool: Some("bash".into()),
        last_error: None,
        pid: 12345,
        socket_path: None,
        parent_id: None,
        workflow: None,
        read_only: false,
        execution_backend: "local".to_string(),
        environment: None,
    };
    let j = round_trip(&info);
    assert_eq!(j["agentId"], "test-agent");
    assert_eq!(j["status"], "running");
    assert_eq!(j["lastTool"], "bash");
    assert_eq!(j["pid"], 12345);
    assert!(j.get("agent_id").is_none());
    assert!(j.get("last_tool").is_none());
    assert!(j.get("lastError").is_none());
}

#[test]
fn subagent_info_null_fields_omitted() {
    let info = SubagentInfo {
        agent_uuid: None,
        display_name: None,
        agent_id: "idle".into(),
        status: "idle".into(),
        liveness: None,
        last_tool: None,
        last_error: None,
        pid: 1,
        socket_path: None,
        parent_id: None,
        workflow: None,
        read_only: false,
        execution_backend: "local".to_string(),
        environment: None,
    };
    let j = round_trip(&info);
    assert!(j.get("lastTool").is_none());
    assert!(j.get("lastError").is_none());
}

#[test]
fn subagent_info_with_error_field() {
    let info = SubagentInfo {
        agent_uuid: None,
        display_name: None,
        agent_id: "err".into(),
        status: "error".into(),
        liveness: None,
        last_tool: None,
        last_error: Some("tool 'bash' returned error".into()),
        pid: 0,
        socket_path: None,
        parent_id: None,
        workflow: None,
        read_only: false,
        execution_backend: "local".to_string(),
        environment: None,
    };
    let j = round_trip(&info);
    assert_eq!(j["lastError"], "tool 'bash' returned error");
}

#[test]
fn subagent_state_changed_event_matches_spec() {
    let ev = AgentEvent::SubagentStateChanged {
        subagents: vec![
            SubagentInfo {
                agent_uuid: None,
                display_name: None,
                agent_id: "reviewer".into(),
                status: "running".into(),
                liveness: None,
                last_tool: Some("bash: cargo test".into()),
                last_error: None,
                pid: 12345,
                socket_path: None,
                parent_id: None,
                workflow: None,
                read_only: false,
                execution_backend: "local".to_string(),
                environment: None,
            },
            SubagentInfo {
                agent_uuid: None,
                display_name: None,
                agent_id: "formatter".into(),
                status: "idle".into(),
                liveness: None,
                last_tool: None,
                last_error: None,
                pid: 12346,
                socket_path: None,
                parent_id: None,
                workflow: None,
                read_only: false,
                execution_backend: "local".to_string(),
                environment: None,
            },
        ],
    };
    let j = round_trip(&ev);
    assert_eq!(j["type"], "subagent_state_changed");
    assert!(j["subagents"].is_array());
    let arr = j["subagents"].as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["agentId"], "reviewer");
    assert_eq!(arr[0]["status"], "running");
    assert_eq!(arr[0]["lastTool"], "bash: cargo test");
    assert_eq!(arr[0]["pid"], 12345);
    assert_eq!(arr[1]["agentId"], "formatter");
    assert_eq!(arr[1]["status"], "idle");
}

#[test]
fn subagent_state_changed_event_roundtrip() {
    let ev = AgentEvent::SubagentStateChanged {
        subagents: vec![SubagentInfo {
            agent_uuid: None,
            display_name: None,
            agent_id: "test".into(),
            status: "exited".into(),
            liveness: None,
            last_tool: None,
            last_error: None,
            pid: 999,
            socket_path: None,
            parent_id: None,
            workflow: None,
            read_only: false,
            execution_backend: "local".to_string(),
            environment: None,
        }],
    };
    let json = ev.to_json_line();
    let parsed: AgentEvent = serde_json::from_str(&json).unwrap();
    match parsed {
        AgentEvent::SubagentStateChanged { subagents } => {
            assert_eq!(subagents.len(), 1);
            assert_eq!(subagents[0].agent_id, "test");
            assert_eq!(subagents[0].status, "exited");
        }
        _ => panic!("expected SubagentStateChanged"),
    }
}

#[test]
fn subagent_state_changed_empty_list() {
    let ev = AgentEvent::SubagentStateChanged { subagents: vec![] };
    let json = ev.to_json_line();
    assert!(json.contains("\"subagents\":[]"));
}

// ─── AgentCommand::type_name() ────────────────────────────────────────────────

#[test]
fn core_command_type_names() {
    assert_eq!(AgentCommand::Abort { id: None }.type_name(), "abort");
    assert_eq!(
        AgentCommand::GetState {
            id: None,
            since: None,
            agent_id: None
        }
        .type_name(),
        "get_state"
    );
    assert_eq!(
        AgentCommand::GetSessionStats { id: None }.type_name(),
        "get_session_stats"
    );
    assert_eq!(
        AgentCommand::SetModel {
            id: None,
            model: Some("m".into()),
            provider: None,
            model_id: None
        }
        .type_name(),
        "set_model"
    );
    assert_eq!(
        AgentCommand::FollowUp {
            id: None,
            message: "m".into()
        }
        .type_name(),
        "follow_up"
    );
    assert_eq!(
        AgentCommand::Steer {
            id: None,
            message: "m".into()
        }
        .type_name(),
        "steer"
    );
    assert_eq!(
        AgentCommand::ClearHistory { id: None }.type_name(),
        "clear_history"
    );
    assert_eq!(
        AgentCommand::GetSubagents {
            id: None,
            since: None
        }
        .type_name(),
        "get_subagents"
    );
    assert_eq!(
        AgentCommand::ListSessions { id: None }.type_name(),
        "list_sessions"
    );
    assert_eq!(
        AgentCommand::ResumeSession {
            id: None,
            session: "work".into(),
        }
        .type_name(),
        "resume_session"
    );
}

// ─── clear_history (#408) ────────────────────────────────────────────────────

#[test]
fn clear_history_command_parses() {
    let json = r#"{"type":"clear_history","id":"ch-1"}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.type_name(), "clear_history");
    assert_eq!(cmd.id(), Some("ch-1"));
}

#[test]
fn clear_history_command_parses_without_id() {
    let json = r#"{"type":"clear_history"}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.type_name(), "clear_history");
    assert!(cmd.id().is_none());
}

#[test]
fn clear_history_command_serializes() {
    let cmd = AgentCommand::ClearHistory {
        id: Some("ch-1".into()),
    };
    let j = round_trip(&cmd);
    assert_eq!(j["type"], "clear_history");
    assert_eq!(j["id"], "ch-1");
}

#[test]
fn clear_history_command_serializes_without_id() {
    let cmd = AgentCommand::ClearHistory { id: None };
    let j = round_trip(&cmd);
    assert_eq!(j["type"], "clear_history");
    assert!(j.get("id").is_none());
}
