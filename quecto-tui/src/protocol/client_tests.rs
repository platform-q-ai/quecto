use super::*;
use tokio::io::AsyncWriteExt;
#[test]
fn command_serializes_to_json_lines() {
    let cmd = Command::Prompt {
        id: Some("p-1".into()),
        message: "hello".into(),
        streaming_behavior: None,
    };
    let json = serde_json::to_string(&cmd).unwrap();
    assert!(json.contains("\"type\":\"prompt\""));
    assert!(json.contains("\"message\":\"hello\""));
    assert!(json.contains("\"id\":\"p-1\""));
    // streaming_behavior should be omitted when None
    assert!(!json.contains("streamingBehavior"));
}
#[test]
fn command_prompt_with_streaming_behavior() {
    let cmd = Command::Prompt {
        id: None,
        message: "hi".into(),
        streaming_behavior: Some("steer".into()),
    };
    let json = serde_json::to_string(&cmd).unwrap();
    assert!(json.contains("\"streamingBehavior\":\"steer\""));
}
#[test]
fn command_get_state_serializes() {
    let cmd = Command::GetState {
        id: Some("gs-1".into()),
    };
    let json = serde_json::to_string(&cmd).unwrap();
    assert!(json.contains("\"type\":\"get_state\""));
}
#[test]
fn command_abort_serializes() {
    let cmd = Command::Abort { id: None };
    let json = serde_json::to_string(&cmd).unwrap();
    assert!(json.contains("\"type\":\"abort\""));
    assert!(!json.contains("\"id\""));
}
#[test]
fn event_deserializes_agent_start() {
    let json = r#"{"type":"agent_start"}"#;
    let event: Event = serde_json::from_str(json).unwrap();
    assert!(matches!(event, Event::AgentStart));
}
#[test]
fn event_deserializes_token() {
    let json = r#"{"type":"token","token":"hello"}"#;
    let event: Event = serde_json::from_str(json).unwrap();
    match event {
        Event::Token { token } => assert_eq!(token, "hello"),
        _ => panic!("expected Token event"),
    }
}
#[test]
fn event_deserializes_response() {
    let json =
        r#"{"type":"response","command":"get_state","success":true,"data":{"model":"test"}}"#;
    let event: Event = serde_json::from_str(json).unwrap();
    match event {
        Event::Response {
            command, success, ..
        } => {
            assert_eq!(command, "get_state");
            assert!(success);
        }
        _ => panic!("expected Response event"),
    }
}
#[test]
fn event_deserializes_tool_execution_start() {
    let json = r#"{"type":"tool_execution_start","toolCallId":"c-1","toolName":"bash","args":{"command":"ls"}}"#;
    let event: Event = serde_json::from_str(json).unwrap();
    match event {
        Event::ToolExecutionStart {
            tool_call_id,
            tool_name,
            ..
        } => {
            assert_eq!(tool_call_id, "c-1");
            assert_eq!(tool_name, "bash");
        }
        _ => panic!("expected ToolExecutionStart"),
    }
}
#[test]
fn event_deserializes_tool_execution_end() {
    let json = r#"{"type":"tool_execution_end","toolCallId":"c-1","toolName":"bash","result":{"content":[]},"isError":false}"#;
    let event: Event = serde_json::from_str(json).unwrap();
    match event {
        Event::ToolExecutionEnd {
            is_error,
            tool_name,
            ..
        } => {
            assert!(!is_error);
            assert_eq!(tool_name, "bash");
        }
        _ => panic!("expected ToolExecutionEnd"),
    }
}
#[test]
fn event_deserializes_subagent_state_changed() {
    let json = r#"{"type":"subagent_state_changed","subagents":[{"agentId":"reviewer","status":"running","lastTool":"bash","pid":123}]}"#;
    let event: Event = serde_json::from_str(json).unwrap();
    match event {
        Event::SubagentStateChanged { subagents } => {
            assert_eq!(subagents.len(), 1);
            assert_eq!(subagents[0].agent_id, "reviewer");
            assert_eq!(subagents[0].status, "running");
            assert_eq!(subagents[0].last_tool.as_deref(), Some("bash"));
            assert_eq!(subagents[0].pid, 123);
        }
        _ => panic!("expected SubagentStateChanged"),
    }
}
#[test]
fn event_deserializes_subagent_notification() {
    let json = r#"{"type":"subagent_notification","agentId":"researcher","sequence":3,"message":"[subagent] Agent 'researcher' completed. Last output: all tests pass"}"#;
    let event: Event = serde_json::from_str(json).unwrap();
    match event {
        Event::SubagentNotification {
            agent_id,
            sequence,
            message,
        } => {
            assert_eq!(agent_id, "researcher");
            assert_eq!(sequence, 3);
            assert!(message.contains("all tests pass"));
        }
        _ => panic!("expected SubagentNotification"),
    }
}
#[test]
fn event_subagent_state_changed_empty() {
    let json = r#"{"type":"subagent_state_changed","subagents":[]}"#;
    let event: Event = serde_json::from_str(json).unwrap();
    match event {
        Event::SubagentStateChanged { subagents } => assert!(subagents.is_empty()),
        _ => panic!("expected SubagentStateChanged"),
    }
}
#[test]
fn event_subagent_info_with_error() {
    let json = r#"{"type":"subagent_state_changed","subagents":[{"agentId":"lint","status":"error","lastError":"tool 'bash' returned error","pid":0}]}"#;
    let event: Event = serde_json::from_str(json).unwrap();
    match event {
        Event::SubagentStateChanged { subagents } => {
            assert_eq!(
                subagents[0].last_error.as_deref(),
                Some("tool 'bash' returned error")
            );
            assert!(subagents[0].last_tool.is_none());
        }
        _ => panic!("expected SubagentStateChanged"),
    }
}
#[test]
fn command_get_subagents_serializes() {
    let cmd = Command::GetSubagents {
        id: Some("gs-1".into()),
    };
    let json = serde_json::to_string(&cmd).unwrap();
    assert!(json.contains("\"type\":\"get_subagents\""));
    assert!(json.contains("\"id\":\"gs-1\""));
}
#[test]
fn event_unknown_type_deserializes_as_unknown() {
    let json = r#"{"type":"some_future_event","data":42}"#;
    let event: Event = serde_json::from_str(json).unwrap();
    assert!(matches!(event, Event::Unknown));
}

#[test]
fn event_deserializes_agent_end() {
    let json = r#"{"type":"agent_end","messages":[{"role":"assistant","content":"hi"}]}"#;
    let event: Event = serde_json::from_str(json).unwrap();
    assert!(matches!(event, Event::AgentEnd { .. }));
}

#[test]
fn agent_end_tolerates_unused_messages_payload() {
    // The wire format still carries `messages`; deserialization must succeed
    // while serde drops the unconsumed field. Retention itself is asserted by
    // the BDD step "undisplayed completion details do not remain in the
    // client event", which fires if the field is ever re-added to the enum.
    let json = r#"{"type":"agent_end","messages":[{"role":"assistant","content":"unused-agent-end-payload"}]}"#;
    let event: Event = serde_json::from_str(json).unwrap();

    assert!(matches!(event, Event::AgentEnd { .. }));
}

// ── Integration: result text extraction ─────────────────────────

#[test]
fn tool_end_read_result_extraction() {
    // Simulate the exact JSON the quecto server sends for a read tool result.
    let json = r#"{"type":"tool_execution_end","toolCallId":"c-1","toolName":"read","result":{"content":[{"type":"text","text":"fn main() {\n    println!(\"hello\");\n}"}]},"isError":false}"#;
    let event: Event = serde_json::from_str(json).unwrap();
    match event {
        Event::ToolExecutionEnd { result, .. } => {
            let text = extract_result_text(&result);
            assert!(
                text.contains("fn main()"),
                "should extract read content: {:?}",
                text
            );
        }
        _ => panic!("expected ToolExecutionEnd"),
    }
}

#[test]
fn tool_end_bash_result_extraction() {
    let json = r#"{"type":"tool_execution_end","toolCallId":"c-1","toolName":"bash","result":{"content":[{"type":"text","text":"file1.txt\nfile2.txt\nfile3.txt"}]},"isError":false}"#;
    let event: Event = serde_json::from_str(json).unwrap();
    match event {
        Event::ToolExecutionEnd { result, .. } => {
            let text = extract_result_text(&result);
            assert!(text.contains("file1.txt"), "should extract bash output");
            assert!(text.contains("file3.txt"), "should have all lines");
        }
        _ => panic!("expected ToolExecutionEnd"),
    }
}

#[test]
fn tool_end_empty_content_array() {
    let json = r#"{"type":"tool_execution_end","toolCallId":"c-1","toolName":"bash","result":{"content":[]},"isError":false}"#;
    let event: Event = serde_json::from_str(json).unwrap();
    match event {
        Event::ToolExecutionEnd { result, .. } => {
            let text = extract_result_text(&result);
            assert_eq!(text, "", "empty content array → empty string");
        }
        _ => panic!("expected ToolExecutionEnd"),
    }
}

#[test]
fn tool_end_edit_result_extraction() {
    let json = r#"{"type":"tool_execution_end","toolCallId":"c-1","toolName":"edit","result":{"content":[{"type":"text","text":"Applied edit to src/main.rs\n+added\n-removed"}]},"isError":false}"#;
    let event: Event = serde_json::from_str(json).unwrap();
    match event {
        Event::ToolExecutionEnd { result, .. } => {
            let text = extract_result_text(&result);
            assert!(text.contains("+added"), "should extract diff content");
            assert!(text.contains("-removed"), "should have diff lines");
        }
        _ => panic!("expected ToolExecutionEnd"),
    }
}

// --- Missing Command serialization tests ---

#[test]
fn command_steer_serializes() {
    let cmd = Command::Steer {
        id: Some("s-1".into()),
        message: "go left".into(),
    };
    let json = serde_json::to_string(&cmd).unwrap();
    assert!(json.contains("\"type\":\"steer\""));
    assert!(json.contains("\"message\":\"go left\""));
}

#[test]
fn command_follow_up_serializes() {
    let cmd = Command::FollowUp {
        id: None,
        message: "also do this".into(),
    };
    let json = serde_json::to_string(&cmd).unwrap();
    assert!(json.contains("\"type\":\"follow_up\""));
    assert!(json.contains("also do this"));
}

#[test]
fn command_set_workflow_automation_serializes() {
    let cmd = Command::SetWorkflowAutomation {
        id: Some("wf".into()),
        auto_continue: Some(false),
        completion_nudge: Some(true),
    };
    let json = serde_json::to_string(&cmd).unwrap();
    assert!(json.contains("\"type\":\"set_workflow_automation\""));
    assert!(json.contains("\"autoContinue\":false"));
    assert!(json.contains("\"completionNudge\":true"));
}

#[test]
fn command_get_messages_serializes() {
    let cmd = Command::GetMessages {
        id: None,
        before: None,
    };
    let json = serde_json::to_string(&cmd).unwrap();
    assert!(json.contains("\"type\":\"get_messages\""));
}

#[test]
fn command_get_messages_tail_serializes() {
    let cmd = Command::GetMessagesTail {
        id: Some("gmt".into()),
        count: 5,
        agent_id: None,
    };
    let json = serde_json::to_string(&cmd).unwrap();
    assert!(json.contains("\"type\":\"get_messages_tail\""));
    assert!(json.contains("\"count\":5"));
    assert!(
        !json.contains("agent_id"),
        "absent agent_id must be omitted"
    );
}

#[test]
fn command_get_messages_tail_serializes_agent_id() {
    let cmd = Command::GetMessagesTail {
        id: Some("gmt".into()),
        count: 1,
        agent_id: Some("worker".into()),
    };
    let json = serde_json::to_string(&cmd).unwrap();
    assert!(json.contains("\"agent_id\":\"worker\""), "got: {json}");
}

#[test]
fn command_get_session_stats_serializes() {
    let cmd = Command::GetSessionStats {
        id: Some("stats".into()),
    };
    let json = serde_json::to_string(&cmd).unwrap();
    assert!(json.contains("\"type\":\"get_session_stats\""));
}

#[test]
fn command_set_model_serializes() {
    let cmd = Command::SetModel {
        id: None,
        model: Some("gpt-4o".into()),
        provider: None,
        model_id: None,
    };
    let json = serde_json::to_string(&cmd).unwrap();
    assert!(json.contains("\"type\":\"set_model\""));
    assert!(json.contains("\"model\":\"gpt-4o\""));
    assert!(!json.contains("provider"));
    assert!(!json.contains("modelId"));
}

#[test]
fn command_set_model_with_provider() {
    let cmd = Command::SetModel {
        id: None,
        model: None,
        provider: Some("anthropic".into()),
        model_id: Some("claude-sonnet-4-6".into()),
    };
    let json = serde_json::to_string(&cmd).unwrap();
    assert!(json.contains("\"provider\":\"anthropic\""));
    assert!(json.contains("\"modelId\":\"claude-sonnet-4-6\""));
}

#[test]
fn command_clear_history_serializes() {
    let cmd = Command::ClearHistory { id: None };
    let json = serde_json::to_string(&cmd).unwrap();
    assert!(json.contains("\"type\":\"clear_history\""));
}

#[test]
fn command_rewind_to_serializes() {
    let cmd = Command::RewindTo {
        id: Some("rw".into()),
        message_id: "m2".into(),
    };
    let json = serde_json::to_string(&cmd).unwrap();
    assert!(json.contains("\"type\":\"rewind_to\""));
    assert!(json.contains("\"messageId\":\"m2\""));
}

#[test]
fn command_list_sessions_serializes() {
    let cmd = Command::ListSessions {
        id: Some("ls".into()),
    };
    let json = serde_json::to_string(&cmd).unwrap();
    assert!(json.contains("\"type\":\"list_sessions\""));
}

#[test]
fn command_resume_session_serializes() {
    let cmd = Command::ResumeSession {
        id: Some("resume".into()),
        session: "work".into(),
    };
    let json = serde_json::to_string(&cmd).unwrap();
    assert!(json.contains("\"type\":\"resume_session\""));
    assert!(json.contains("\"session\":\"work\""));
}

// --- Event deserialization edge cases ---

#[test]
fn event_deserializes_turn_start() {
    let json = r#"{"type":"turn_start"}"#;
    let event: Event = serde_json::from_str(json).unwrap();
    assert!(matches!(event, Event::TurnStart));
}

#[test]
fn event_deserializes_turn_end() {
    let json =
        r#"{"type":"turn_end","message":{"role":"assistant","content":"hi"},"toolResults":[]}"#;
    let event: Event = serde_json::from_str(json).unwrap();
    match event {
        Event::TurnEnd { message } => {
            assert_eq!(message["role"], "assistant");
        }
        _ => panic!("expected TurnEnd"),
    }
}

#[test]
fn turn_end_keeps_message_and_tolerates_unused_tool_results() {
    // See agent_end_tolerates_unused_messages_payload for why retention is
    // not re-asserted here.
    let json = r#"{"type":"turn_end","message":{"role":"assistant","content":"kept"},"toolResults":[{"content":[{"type":"text","text":"unused-turn-end-payload"}]}]}"#;
    let event: Event = serde_json::from_str(json).unwrap();

    match &event {
        Event::TurnEnd { message } => assert_eq!(message["content"], "kept"),
        _ => panic!("expected TurnEnd"),
    }
}

#[test]
fn event_deserializes_tool_catalogue_changed() {
    let json = r#"{"type":"tool_catalogue_changed","changedTools":["weather"],"before":[],"after":[{"name":"weather","source":"uds"}],"reason":"register_tool"}"#;
    let event: Event = serde_json::from_str(json).unwrap();
    match event {
        Event::ToolCatalogueChanged {
            changed_tools,
            before,
            after,
            reason,
        } => {
            assert_eq!(changed_tools, vec!["weather"]);
            assert!(before.is_empty());
            assert_eq!(after[0].name, "weather");
            assert_eq!(reason, "register_tool");
        }
        other => panic!("expected tool catalogue event, got {other:?}"),
    }
}

#[test]
fn event_response_with_error() {
    let json =
        r#"{"type":"response","command":"set_model","success":false,"error":"model not found"}"#;
    let event: Event = serde_json::from_str(json).unwrap();
    match event {
        Event::Response { success, error, .. } => {
            assert!(!success);
            assert_eq!(error.as_deref(), Some("model not found"));
        }
        _ => panic!("expected Response"),
    }
}

// --- extract_result_text edge cases ---

#[test]
fn extract_result_text_no_content_key() {
    let val = serde_json::json!({"something": "else"});
    assert_eq!(extract_result_text(&val), "");
}

#[test]
fn extract_result_text_non_text_type() {
    let val = serde_json::json!({"content": [{"type": "image", "data": "abc"}]});
    assert_eq!(extract_result_text(&val), "");
}

// --- ClientError Display ---

#[test]
fn client_error_display() {
    let e = ClientError::Disconnected;
    assert_eq!(e.to_string(), "disconnected from agent");
}

#[test]
fn client_error_from_io() {
    let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "not found");
    let e = ClientError::from(io_err);
    assert!(e.to_string().contains("I/O error"));
}

#[test]
fn client_error_from_json() {
    let json_err: serde_json::Error = serde_json::from_str::<Event>("bad json").unwrap_err();
    let e = ClientError::from(json_err);
    assert!(e.to_string().contains("JSON error"));
}

#[tokio::test]
async fn client_send_serializes_command_line() {
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel(1);
    let (_event_tx, event_rx) = tokio::sync::mpsc::channel(1);
    let mut client = Client {
        cmd_tx,
        event_rx,
        dropped_oversized: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
    };

    client
        .send(&Command::GetState {
            id: Some("state-1".into()),
        })
        .await
        .unwrap();

    let line = cmd_rx.recv().await.unwrap();
    assert!(line.ends_with('\n'));
    assert!(line.contains(r#""type":"get_state""#));
    assert!(line.contains(r#""id":"state-1""#));
}

#[tokio::test]
async fn command_sender_send_serializes_command_line() {
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel(1);
    let (_event_tx, event_rx) = tokio::sync::mpsc::channel(1);
    let client = Client {
        cmd_tx,
        event_rx,
        dropped_oversized: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
    };
    let mut sender = client.clone_sender();

    sender
        .send(&Command::Abort {
            id: Some("abort-1".into()),
        })
        .await
        .unwrap();

    let line = cmd_rx.recv().await.unwrap();
    assert!(line.ends_with('\n'));
    assert!(line.contains(r#""type":"abort""#));
    assert!(line.contains(r#""id":"abort-1""#));
}

#[tokio::test]
async fn client_recv_and_try_recv_return_events() {
    let (cmd_tx, _cmd_rx) = tokio::sync::mpsc::channel(1);
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(2);
    let mut client = Client {
        cmd_tx,
        event_rx,
        dropped_oversized: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
    };

    event_tx
        .send(Event::Token { token: "hi".into() })
        .await
        .unwrap();
    match client.try_recv() {
        Some(Event::Token { token }) => assert_eq!(token, "hi"),
        other => panic!("unexpected event: {other:?}"),
    }

    event_tx
        .send(Event::Response {
            id: Some("r-1".into()),
            command: "get_state".into(),
            success: false,
            data: None,
            error: Some("boom".into()),
        })
        .await
        .unwrap();
    match client.recv().await {
        Some(Event::Response { error, .. }) => assert_eq!(error.as_deref(), Some("boom")),
        other => panic!("unexpected event: {other:?}"),
    }
}

#[tokio::test]
async fn client_send_reports_disconnected_when_command_channel_closed() {
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel(1);
    drop(cmd_rx);
    let (_event_tx, event_rx) = tokio::sync::mpsc::channel(1);
    let mut client = Client {
        cmd_tx,
        event_rx,
        dropped_oversized: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
    };

    let err = client
        .send(&Command::GetSubagents { id: None })
        .await
        .unwrap_err();
    assert!(matches!(err, ClientError::Disconnected));
}

#[tokio::test]
async fn client_connect_reads_events_and_writes_commands() {
    let dir = std::env::temp_dir().join(format!("quecto-tui-client-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let socket_path = dir.join("agent.sock");
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = tokio::io::split(stream);
        // A blank keep-alive line, then a `{`-opening line that is not valid
        // JSON: both must be tolerated (skipped) by the deprecation-window
        // sniffing reader (#1059) without killing the connection.
        write_half.write_all(b"\n{not json\n").await.unwrap();
        write_half
            .write_all(br#"{"type":"token","token":"from-server"}"#)
            .await
            .unwrap();
        write_half.write_all(b"\n").await.unwrap();
        write_half.flush().await.unwrap();

        // Decode the wire STRICTLY as length-prefixed frames (4-byte BE length
        // + payload) rather than via the dual-accepting sniffing reader, so a
        // revert of the client writer to legacy NDJSON is caught: a `{...}\n`
        // line would be misread as a ~1.9 GiB frame length and stall/fail
        // instead of passing (#1059 — pins the framed-WRITE migration).
        use tokio::io::AsyncReadExt;
        let mut reader = tokio::io::BufReader::new(read_half);
        let mut prefix = [0u8; 4];
        // First frame is the empty hello frame announcing framed mode.
        reader.read_exact(&mut prefix).await.unwrap();
        assert_eq!(u32::from_be_bytes(prefix), 0, "expected empty hello frame");
        reader.read_exact(&mut prefix).await.unwrap();
        let declared = u32::from_be_bytes(prefix) as usize;
        assert!(
            declared > 0 && declared <= quecto_line_io::PROTOCOL_FRAME_CAP_BYTES,
            "implausible command frame length ({declared}) — writer is not framing"
        );
        let mut payload = vec![0u8; declared];
        reader.read_exact(&mut payload).await.unwrap();
        let command = String::from_utf8(payload).unwrap();
        assert!(
            !command.ends_with('\n'),
            "framed payload must have no newline"
        );
        command
    });

    let mut client = Client::connect(&socket_path).await.unwrap();
    client
        .send(&Command::SetModel {
            id: Some("m-1".into()),
            model: Some("test-model".into()),
            provider: None,
            model_id: None,
        })
        .await
        .unwrap();

    match tokio::time::timeout(std::time::Duration::from_secs(1), client.recv())
        .await
        .unwrap()
    {
        Some(Event::Token { token }) => assert_eq!(token, "from-server"),
        other => panic!("unexpected event: {other:?}"),
    }

    let written = server.await.unwrap();
    assert!(written.contains(r#""type":"set_model""#));
    assert!(written.contains(r#""model":"test-model""#));
}

#[tokio::test]
async fn client_connect_returns_io_error_for_missing_socket() {
    let path = std::env::temp_dir().join(format!(
        "quecto-tui-missing-client-test-{}.sock",
        std::process::id()
    ));
    let err = match Client::connect(&path).await {
        Ok(_) => panic!("missing socket unexpectedly connected"),
        Err(err) => err,
    };
    assert!(matches!(err, ClientError::Io(_)));
}

#[test]
fn forwarded_canonical_workflow_state_parses_without_steps() {
    // PRD Stage B forwards child workflow_state canonically (no `steps`). It must
    // still deserialize as WorkflowState (not error → which would print raw JSON
    // over the TUI, the "percent N" leak).
    let line = r#"{"type":"workflow_state","agent_id":"tui-ui-test-3","parent_id":null,"mode":"active","progress":{"done":3,"total":5,"percent":60}}"#;
    let ev: super::Event = serde_json::from_str(line).expect("must parse, not error");
    match ev {
        super::Event::WorkflowState { agent_id, .. } => {
            assert_eq!(agent_id.as_deref(), Some("tui-ui-test-3"));
        }
        other => panic!("expected WorkflowState, got {other:?}"),
    }
}

#[test]
fn serialize_command_appends_single_newline() {
    let cmd = Command::Abort {
        id: Some("a-1".into()),
    };
    let framed = serialize_command(&cmd).expect("serialize");
    assert!(framed.ends_with('\n'), "wire frame must end with a newline");
    assert_eq!(
        framed.matches('\n').count(),
        1,
        "exactly one trailing newline frames the JSON line"
    );
    // The framed body must be the command's JSON plus that newline.
    assert_eq!(
        framed,
        format!("{}\n", serde_json::to_string(&cmd).unwrap())
    );
}

#[tokio::test]
async fn command_sender_and_client_send_emit_identical_bytes() {
    // Both senders write through serialize_command, so the same command must
    // appear on the wire byte-for-byte regardless of which path sent it (#759).
    let (tx, mut rx) = mpsc::channel::<String>(4);
    let mut sender = CommandSender { tx };
    let cmd = Command::SetModel {
        id: Some("m-1".into()),
        model: Some("claude".into()),
        provider: None,
        model_id: None,
    };
    sender.send(&cmd).await.expect("send");
    let from_sender = rx.recv().await.expect("frame");

    let expected = serialize_command(&cmd).expect("serialize");
    assert_eq!(from_sender, expected);
}

#[test]
fn workspace_event_deserializes_1350() {
    let ev: super::Event = serde_json::from_str(r#"{"type":"workspace","path":"/tmp/ws"}"#).unwrap();
    match ev {
        super::Event::Workspace { path } => assert_eq!(path, "/tmp/ws"),
        other => panic!("expected workspace event, got {other:?}"),
    }
}
