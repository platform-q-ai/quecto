use super::*;

// ─── AgentCommand deserialization ──────────────────────────────────────────

#[test]
fn test_parse_prompt_command() {
    let json = r#"{"type":"prompt","message":"hello world"}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    match cmd {
        AgentCommand::Prompt {
            message,
            id,
            streaming_behavior,
        } => {
            assert_eq!(message, "hello world");
            assert!(id.is_none());
            assert!(streaming_behavior.is_none());
        }
        _ => panic!("expected Prompt"),
    }
}

#[test]
fn test_parse_prompt_with_id() {
    let json = r#"{"type":"prompt","id":"req-1","message":"hello"}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.id(), Some("req-1"));
}

#[test]
fn test_parse_prompt_with_steer_behavior() {
    let json = r#"{"type":"prompt","message":"hi","streamingBehavior":"steer"}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    match cmd {
        AgentCommand::Prompt {
            streaming_behavior, ..
        } => {
            assert_eq!(streaming_behavior, Some(StreamingBehavior::Steer));
        }
        _ => panic!("expected Prompt"),
    }
}

#[test]
fn test_parse_prompt_with_follow_up_behavior() {
    let json = r#"{"type":"prompt","message":"hi","streamingBehavior":"followUp"}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    match cmd {
        AgentCommand::Prompt {
            streaming_behavior, ..
        } => {
            assert_eq!(streaming_behavior, Some(StreamingBehavior::FollowUp));
        }
        _ => panic!("expected Prompt"),
    }
}

#[test]
fn test_parse_steer_command() {
    let json = r#"{"type":"steer","message":"change direction"}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    match cmd {
        AgentCommand::Steer { message, .. } => assert_eq!(message, "change direction"),
        _ => panic!("expected Steer"),
    }
}

#[test]
fn test_parse_follow_up_command() {
    let json = r#"{"type":"follow_up","message":"also do this"}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    match cmd {
        AgentCommand::FollowUp { message, .. } => assert_eq!(message, "also do this"),
        _ => panic!("expected FollowUp"),
    }
}

#[test]
fn test_parse_abort_command() {
    let json = r#"{"type":"abort"}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    matches!(cmd, AgentCommand::Abort { .. });
}

#[test]
fn test_parse_set_workflow_automation_command() {
    let json = r#"{"type":"set_workflow_automation","autoContinue":false,"completionNudge":true}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    match cmd {
        AgentCommand::SetWorkflowAutomation {
            auto_continue,
            completion_nudge,
            ..
        } => {
            assert_eq!(auto_continue, Some(false));
            assert_eq!(completion_nudge, Some(true));
        }
        _ => panic!("expected SetWorkflowAutomation"),
    }
}

#[test]
fn test_parse_get_state_command() {
    let json = r#"{"type":"get_state","id":"gs-1"}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.id(), Some("gs-1"));
    assert_eq!(cmd.type_name(), "get_state");
}

#[test]
fn test_parse_get_messages_command() {
    let json = r#"{"type":"get_messages"}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.type_name(), "get_messages");
}

#[test]
fn test_parse_get_messages_with_count_command() {
    let json = r#"{"type":"get_messages","id":"gm-1","count":5}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.id(), Some("gm-1"));
    assert_eq!(cmd.type_name(), "get_messages");
    let wire = serde_json::to_value(&cmd).unwrap();
    assert_eq!(wire["count"], 5);
}

#[test]
fn test_parse_get_messages_tail_command() {
    let json = r#"{"type":"get_messages_tail","count":5}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    match cmd {
        AgentCommand::GetMessagesTail {
            id,
            count,
            agent_id,
        } => {
            assert!(id.is_none());
            assert_eq!(count, 5);
            assert!(agent_id.is_none());
        }
        _ => panic!("expected GetMessagesTail"),
    }
}

#[test]
fn test_parse_get_messages_tail_with_id() {
    let json = r#"{"type":"get_messages_tail","id":"gmt-1","count":10}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.id(), Some("gmt-1"));
    assert_eq!(cmd.type_name(), "get_messages_tail");
}

#[test]
fn test_parse_get_messages_tail_count_zero() {
    let json = r#"{"type":"get_messages_tail","count":0}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    match cmd {
        AgentCommand::GetMessagesTail { count, .. } => assert_eq!(count, 0),
        _ => panic!("expected GetMessagesTail"),
    }
}

#[test]
fn get_message_range_request_round_trips_wire_fields() {
    let json =
        r#"{"type":"get_message","id":"gm-page-1","messageId":"msg-1","offset":4096,"limit":8192}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.type_name(), "get_message");

    let wire = serde_json::to_value(&cmd).unwrap();
    assert_eq!(wire["messageId"], "msg-1");
    assert_eq!(
        wire["offset"], 4096,
        "range start must survive command parsing"
    );
    assert_eq!(
        wire["limit"], 8192,
        "range length must survive command parsing"
    );
}

#[test]
fn get_message_tool_call_argument_range_round_trips_selector() {
    let json = r#"{"type":"get_message","id":"gm-tool-page","messageId":"msg-1","toolCallId":"call-1","offset":7,"limit":11}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();

    let wire = serde_json::to_value(&cmd).unwrap();
    assert_eq!(wire["messageId"], "msg-1");
    assert_eq!(wire["toolCallId"], "call-1");
    assert_eq!(wire["offset"], 7);
    assert_eq!(wire["limit"], 11);
}

#[test]
fn get_message_range_request_preserves_agent_target() {
    let json = r#"{"type":"get_message","id":"gm-child-page","messageId":"msg-2","agent_id":"worker","offset":12,"limit":34}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();

    let wire = serde_json::to_value(&cmd).unwrap();
    assert_eq!(wire["agent_id"], "worker");
    assert_eq!(
        wire["offset"], 12,
        "child forwarding requires the byte offset"
    );
    assert_eq!(
        wire["limit"], 34,
        "child forwarding requires the byte limit"
    );
}

#[test]
fn test_parse_get_session_stats_command() {
    let json = r#"{"type":"get_session_stats"}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.type_name(), "get_session_stats");
}

#[test]
fn test_parse_set_model_command() {
    let json = r#"{"type":"set_model","model":"gpt-5-mini"}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    match cmd {
        AgentCommand::SetModel {
            model,
            provider,
            model_id,
            ..
        } => {
            assert_eq!(model.as_deref(), Some("gpt-5-mini"));
            assert!(provider.is_none());
            assert!(model_id.is_none());
        }
        _ => panic!("expected SetModel"),
    }
}

#[test]
fn test_parse_set_model_provider_and_model_id_command() {
    let json = r#"{"type":"set_model","provider":"openai-codex","modelId":"gpt-5.3-codex"}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    match cmd {
        AgentCommand::SetModel {
            model,
            provider,
            model_id,
            ..
        } => {
            assert!(model.is_none());
            assert_eq!(provider.as_deref(), Some("openai-codex"));
            assert_eq!(model_id.as_deref(), Some("gpt-5.3-codex"));
        }
        _ => panic!("expected SetModel"),
    }
}

#[test]
fn test_parse_get_subagents_command() {
    let json = r#"{"type":"get_subagents","id":"gs-1"}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.type_name(), "get_subagents");
    assert_eq!(cmd.id(), Some("gs-1"));
}

#[test]
fn test_parse_get_subagents_without_id() {
    let json = r#"{"type":"get_subagents"}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.type_name(), "get_subagents");
    assert!(cmd.id().is_none());
}

/// A minimal local-backend `SubagentInfo` for serialization tests.
fn base_subagent_info(agent_id: &str, status: &str) -> SubagentInfo {
    SubagentInfo {
        agent_uuid: None,
        display_name: None,
        agent_id: agent_id.to_string(),
        status: status.to_string(),
        liveness: None,
        last_tool: None,
        last_error: None,
        pid: 0,
        socket_path: None,
        parent_id: None,
        workflow: None,
        read_only: false,
        execution_backend: "local".to_string(),
        environment: None,
    }
}

#[test]
fn test_subagent_state_changed_event_serializes() {
    let ev = AgentEvent::SubagentStateChanged {
        subagents: vec![SubagentInfo {
            last_tool: Some("bash".to_string()),
            pid: 123,
            ..base_subagent_info("test", "running")
        }],
    };
    let json = ev.to_json_line();
    assert!(json.contains("\"type\":\"subagent_state_changed\""));
    assert!(json.contains("\"agentId\":\"test\""));
    assert!(json.contains("\"status\":\"running\""));
    assert!(json.contains("\"lastTool\":\"bash\""));
}

#[test]
fn test_subagent_messages_appended_event_serializes() {
    let ev = AgentEvent::SubagentMessagesAppended {
        agent_id: String::new(),
        messages: vec![],
        message_refs: vec!["aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into()],
    };
    let json = ev.to_json_line();
    assert!(json.contains("\"type\":\"subagent_messages_appended\""));
    assert!(json.contains("\"agent_id\":\"\""));
    assert!(json.contains("\"messageRefs\""));
}

#[test]
fn test_subagent_info_null_fields_omitted() {
    let info = SubagentInfo {
        pid: 456,
        ..base_subagent_info("idle-agent", "idle")
    };
    let json = serde_json::to_string(&info).unwrap();
    assert!(!json.contains("lastTool"));
    assert!(!json.contains("lastError"));
}

#[test]
fn test_subagent_info_with_error() {
    let info = SubagentInfo {
        last_error: Some("connection refused".to_string()),
        ..base_subagent_info("err", "error")
    };
    let json = serde_json::to_string(&info).unwrap();
    assert!(json.contains("\"lastError\":\"connection refused\""));
}

#[test]
fn test_build_subagent_info_list_empty_registry() {
    let list = build_subagent_info_list(&None);
    assert!(list.is_empty());
}

#[test]
fn test_build_subagent_info_list_with_entries() {
    use crate::infrastructure::tools::subagent_registry::{
        SubagentEntry, SubagentStatus, new_registry,
    };
    let reg = new_registry();
    {
        let mut guard = reg.lock().unwrap();
        let mut entry = SubagentEntry::new("/tmp/a.sock".into(), 1234);
        entry.status = SubagentStatus::Running;
        entry.last_tool = Some("bash".to_string());
        guard.insert("reviewer".to_string(), entry);
        let mut entry2 = SubagentEntry::new("/tmp/b.sock".into(), 5678);
        entry2.status = SubagentStatus::Idle;
        guard.insert("formatter".to_string(), entry2);
    }
    let list = build_subagent_info_list(&Some(reg));
    assert_eq!(list.len(), 2);
    // Sorted by agent_id
    assert_eq!(list[0].agent_id, "formatter");
    assert_eq!(list[0].status, "idle");
    assert_eq!(list[1].agent_id, "reviewer");
    assert_eq!(list[1].status, "running");
    assert_eq!(list[1].last_tool.as_deref(), Some("bash"));
    assert_eq!(list[1].pid, 1234);
}

#[test]
fn test_build_subagent_info_list_maps_all_statuses() {
    use crate::infrastructure::tools::subagent_registry::{
        SubagentEntry, SubagentStatus, new_registry,
    };
    let reg = new_registry();
    {
        let mut guard = reg.lock().unwrap();
        for (id, status) in [
            ("a", SubagentStatus::Starting),
            ("b", SubagentStatus::Idle),
            ("c", SubagentStatus::Running),
            ("d", SubagentStatus::Error),
            ("e", SubagentStatus::Exited),
        ] {
            let mut entry = SubagentEntry::new("/tmp/x.sock".into(), 1);
            entry.status = status;
            guard.insert(id.to_string(), entry);
        }
    }
    let list = build_subagent_info_list(&Some(reg));
    assert_eq!(list[0].status, "starting");
    assert_eq!(list[1].status, "idle");
    assert_eq!(list[2].status, "running");
    assert_eq!(list[3].status, "error");
    assert_eq!(list[4].status, "exited");
}

#[test]
fn test_build_subagent_info_list_omits_socket_path() {
    // #1442: inspection is routed through the root/nearest reachable ancestor;
    // public topology snapshots must not expose raw UDS socket paths.
    use crate::infrastructure::tools::subagent_registry::{
        SubagentEntry, SubagentStatus, new_registry,
    };
    let reg = new_registry();
    {
        let mut guard = reg.lock().unwrap();
        let mut entry = SubagentEntry::new("/run/quecto/worker.sock".into(), 99);
        entry.status = SubagentStatus::Running;
        guard.insert("worker".to_string(), entry);
    }
    let list = build_subagent_info_list(&Some(reg));
    assert_eq!(list.len(), 1);
    assert_eq!(
        list[0].socket_path.as_deref(),
        None,
        "socket_path must not be exposed on public topology snapshots"
    );
}

#[test]
fn test_subagent_info_socket_path_is_backcompat_input_only() {
    // The wire type still accepts older snapshots containing socketPath, but
    // newly built public snapshots omit it.
    use crate::infrastructure::tools::subagent_registry::{
        SubagentEntry, SubagentStatus, new_registry,
    };
    let reg = new_registry();
    {
        let mut guard = reg.lock().unwrap();
        let mut entry = SubagentEntry::new("/run/quecto/worker.sock".into(), 99);
        entry.status = SubagentStatus::Running;
        guard.insert("worker".to_string(), entry);
    }
    let list = build_subagent_info_list(&Some(reg));
    let json = serde_json::to_string(&list[0]).unwrap();
    assert!(
        !json.contains("socketPath"),
        "new public wire form must omit socketPath, got: {json}"
    );
    let back: SubagentInfo =
        serde_json::from_str(r#"{"agentId":"w","status":"idle","pid":1,"socketPath":"/old.sock"}"#)
            .unwrap();
    assert_eq!(back.socket_path.as_deref(), Some("/old.sock"));
}

#[test]
fn test_malformed_json_fails() {
    let result: Result<AgentCommand, _> = serde_json::from_str("not json{");
    assert!(result.is_err());
}

#[test]
fn test_unknown_type_fails() {
    let result: Result<AgentCommand, _> = serde_json::from_str(r#"{"type":"unknown_command"}"#);
    assert!(result.is_err());
}

// ─── AgentEvent serialization ──────────────────────────────────────────────

#[test]
fn test_agent_start_event_serializes() {
    let event = AgentEvent::AgentStart;
    let json = event.to_json_line();
    assert!(json.contains("\"type\":\"agent_start\""));
}

#[test]
fn test_agent_end_event_serializes() {
    let event = AgentEvent::AgentEnd {
        messages: vec![],
        message_refs: vec![],
    };
    let json = event.to_json_line();
    assert!(json.contains("\"type\":\"agent_end\""));
    assert!(json.contains("\"messages\""));
}

#[test]
fn test_turn_start_event_serializes() {
    let event = AgentEvent::TurnStart;
    let json = event.to_json_line();
    assert!(json.contains("\"type\":\"turn_start\""));
}

#[test]
fn test_tool_execution_start_event_serializes() {
    let event = AgentEvent::ToolExecutionStart {
        tool_call_id: "call-1".to_string(),
        tool_name: "bash".to_string(),
        args: serde_json::json!({"command": "echo hi"}),
    };
    let json = event.to_json_line();
    assert!(json.contains("\"type\":\"tool_execution_start\""));
    assert!(json.contains("\"toolName\":\"bash\""));
    assert!(json.contains("\"toolCallId\":\"call-1\""));
}

#[test]
fn test_tool_execution_end_event_serializes() {
    let event = AgentEvent::ToolExecutionEnd {
        tool_call_id: "call-1".to_string(),
        tool_name: "bash".to_string(),
        result: ToolResultContent {
            content: vec![serde_json::json!({"type":"text","text":"hi"})],
        },
        is_error: false,
    };
    let json = event.to_json_line();
    assert!(json.contains("\"type\":\"tool_execution_end\""));
    assert!(json.contains("\"isError\":false"));
}

#[test]
fn test_response_ok_event_serializes() {
    let event = AgentEvent::ok(Some("req-1"), "prompt", None);
    let json = event.to_json_line();
    assert!(json.contains("\"type\":\"response\""));
    assert!(json.contains("\"command\":\"prompt\""));
    assert!(json.contains("\"success\":true"));
    assert!(json.contains("\"id\":\"req-1\""));
}

#[test]
fn test_response_err_event_serializes() {
    let event = AgentEvent::err(None, "prompt", "agent already running");
    let json = event.to_json_line();
    assert!(json.contains("\"success\":false"));
    assert!(json.contains("\"error\":\"agent already running\""));
    // id field should be absent when None
    assert!(!json.contains("\"id\""));
}

#[test]
fn test_response_without_id_omits_id_field() {
    let event = AgentEvent::ok(None, "abort", None);
    let json = event.to_json_line();
    assert!(!json.contains("\"id\""));
}

// ─── SessionState / SessionStats ────────────────────────────────────────

#[test]
fn test_session_state_serializes() {
    let state = SessionState {
        execution: None,
        model: "gpt-5".to_string(),
        generation: 1,
        is_streaming: false,
        session_key: "cli:test".to_string(),
        message_count: 4,
        pending_message_count: 0,
        max_context_tokens: 200_000,
        effort: None,
        effort_levels: Vec::new(),
        workflow: None,
        sync: 1,
    };
    let json = serde_json::to_string(&state).unwrap();
    assert!(json.contains("\"isStreaming\":false"));
    assert!(json.contains("\"sessionKey\":\"cli:test\""));
    assert!(json.contains("\"messageCount\":4"));
}

#[test]
fn test_session_state_with_workflow_serializes() {
    let state = SessionState {
        execution: None,
        model: "gpt-5".to_string(),
        generation: 1,
        is_streaming: false,
        session_key: "cli:wf".to_string(),
        message_count: 2,
        pending_message_count: 0,
        max_context_tokens: 200_000,
        effort: None,
        effort_levels: Vec::new(),
        workflow: Some(serde_json::json!({
            "enabled": true,
            "guardsEnabled": true,
            "mode": "active",
            "progress": { "done": 1, "total": 7, "percent": 14 }
        })),
        sync: 1,
    };
    let json = serde_json::to_string(&state).unwrap();
    assert!(json.contains("\"workflow\""));
    assert!(json.contains("\"active\""));
}

#[test]
fn test_session_state_without_workflow_omits_field() {
    let state = SessionState {
        execution: None,
        model: "gpt-5".to_string(),
        generation: 1,
        is_streaming: false,
        session_key: "cli:no_wf".to_string(),
        message_count: 0,
        pending_message_count: 0,
        max_context_tokens: 200_000,
        effort: None,
        effort_levels: Vec::new(),
        workflow: None,
        sync: 1,
    };
    let json = serde_json::to_string(&state).unwrap();
    assert!(
        !json.contains("workflow"),
        "disabled workflow should be absent from get_state"
    );
}

#[test]
fn test_workflow_state_event_serializes() {
    let event = AgentEvent::WorkflowState {
        enabled: true,
        guards_enabled: true,
        mode: "selecting_template".to_string(),
        active_template: None,
        active_issue: None,
        progress: serde_json::json!({"done": 0, "total": 0, "percent": 0}),
        current_step: None,
        steps: vec![],
        available_templates: vec![serde_json::json!({"id": "feature", "label": "Feature"})],
    };
    let json = event.to_json_line();
    assert!(json.contains("\"type\":\"workflow_state\""));
    assert!(json.contains("\"selecting_template\""));
    assert!(json.contains("\"feature\""));
}

#[test]
fn test_session_stats_serializes() {
    let stats = SessionStats {
        session_key: "cli:test".to_string(),
        user_messages: 2,
        assistant_messages: 2,
        tool_calls: 3,
        tool_results: 3,
        total_messages: 10,
        tokens: TokenStats {
            input: 1000,
            output: 200,
            cache_read: 800,
            cache_write: 100,
            total: 1200,
        },
        cost: 0.42,
        cost_micro_usd: 420_000,
        cache_hit_ratio: Some(800.0 / 1900.0),
        context_tokens: 12_000,
        max_context_tokens: 200_000,
    };
    let json = serde_json::to_string(&stats).unwrap();
    assert!(json.contains("\"userMessages\":2"));
    assert!(json.contains("\"totalMessages\":10"));
    assert!(json.contains("\"tokens\""));
    assert!(json.contains("\"costMicroUsd\":420000"));
    assert!(json.contains("\"cacheHitRatio\""));
    assert!(json.contains("\"contextTokens\":12000"));
}

#[test]
fn test_session_stats_deserializes_without_cost() {
    let json = r#"{
        "sessionKey": "cli:test",
        "userMessages": 2,
        "assistantMessages": 2,
        "toolCalls": 3,
        "toolResults": 3,
        "totalMessages": 10,
        "tokens": {
            "input": 1000,
            "output": 200,
            "cacheRead": 800,
            "cacheWrite": 100,
            "total": 1200
        },
        "contextTokens": 12000,
        "maxContextTokens": 200000
    }"#;

    let stats: SessionStats = serde_json::from_str(json)
        .expect("SessionStats should deserialize no-cost get_session_stats JSON");

    assert_eq!(stats.session_key, "cli:test");
    assert_eq!(stats.tokens.total, 1200);
    assert_eq!(stats.cost, 0.0);
    assert_eq!(stats.cost_micro_usd, 0);
    assert_eq!(stats.cache_hit_ratio, None);
}

#[test]
fn unit_tree_reconstructs_parentage_from_events() {
    let events = vec![
        serde_json::json!({"agent_id":"root","parent_id":null}),
        serde_json::json!({"agent_id":"child","parent_id":"root"}),
        serde_json::json!({"agent_id":"grandchild","parent_id":"child"}),
    ];
    let tree = UnitTree::from_events(&events);
    assert_eq!(tree.parent_of("grandchild"), Some("child"));
    assert_eq!(tree.parent_of("child"), Some("root"));
    assert_eq!(tree.parent_of("root"), None);
}

#[test]
fn build_subagent_info_list_includes_parent_and_workflow() {
    use crate::infrastructure::tools::subagent_registry::{
        SubagentEntry, WorkflowSnapshot, new_registry,
    };
    let reg = new_registry();
    {
        let mut g = reg.lock().unwrap();
        let mut e = SubagentEntry::new(std::path::PathBuf::from("/tmp/c.sock"), 0);
        e.parent_id = Some("root".to_string());
        e.workflow = Some(WorkflowSnapshot {
            mode: "active".to_string(),
            steps_completed: 1,
            steps_total: 2,
        });
        g.insert("child".to_string(), e);
    }
    let list = build_subagent_info_list(&Some(reg));
    let info = list.iter().find(|i| i.agent_id == "child").unwrap();
    assert_eq!(info.parent_id.as_deref(), Some("root"));
    assert_eq!(info.workflow.as_ref().unwrap().steps_completed, 1);
}

#[test]
fn unit_tree_parent_of_unknown_agent_is_none() {
    let tree = UnitTree::from_events(&[serde_json::json!({"agent_id":"root","parent_id":null})]);
    assert_eq!(tree.parent_of("nope"), None);
    assert_eq!(tree.parent_of("root"), None);
}

#[test]
fn workspace_event_serializes_1350() {
    let event = AgentEvent::Workspace {
        path: "/tmp/ws".into(),
    };
    let json = event.to_json_line();
    assert!(json.contains("\"type\":\"workspace\""), "{json}");
    assert!(json.contains("\"path\":\"/tmp/ws\""), "{json}");
}
