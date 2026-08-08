//! Unit tests for the UDS gateway client (split out to respect file-size limits).
use super::*;
use tokio::io::AsyncWriteExt;

#[tokio::test]
async fn oversized_event_line_is_dropped_but_later_valid_events_still_arrive() {
    let dir = tempfile::tempdir().expect("tempdir");
    let socket_path = dir.path().join("agent.sock");
    let listener = tokio::net::UnixListener::bind(&socket_path).expect("bind test socket");

    // Gate the server writes on the subscription existing: broadcast
    // sends before `subscribe()` are dropped (see the reader loop in
    // `connect`), so writing immediately after accept races the
    // subscription and flakes.
    let (subscribed_tx, subscribed_rx) = tokio::sync::oneshot::channel::<()>();
    let accept_task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        subscribed_rx.await.expect("subscribed signal");
        // One giant unterminated-then-terminated line, well over
        // MAX_LINE_BYTES, followed by a normal, valid event.
        let oversized = format!(
            "{{\"type\":\"token\",\"token\":\"{}\"}}\n",
            "x".repeat(MAX_LINE_BYTES + 65_536)
        );
        stream
            .write_all(oversized.as_bytes())
            .await
            .expect("write oversized line");
        stream
            .write_all(b"{\"type\":\"token\",\"token\":\"hi\"}\n")
            .await
            .expect("write valid line");
    });

    let gateway = UdsGateway::connect(&socket_path)
        .await
        .expect("connect to agent socket");
    let mut sub = gateway.subscribe().await.expect("subscribe");
    subscribed_tx.send(()).expect("signal subscribed");

    let event = tokio::time::timeout(std::time::Duration::from_secs(3), sub.recv())
        .await
        .expect("subscriber should still receive an event after an oversized line within 3s")
        .expect("event present");

    match event {
        AgentEvent::Token { token } => assert_eq!(
            token, "hi",
            "the oversized line must not be delivered as a parsed event, only the valid one that follows it"
        ),
        other => panic!("expected a Token event, got: {other:?}"),
    }

    accept_task.await.expect("accept task completed");
}

#[test]
fn max_line_bytes_matches_documented_protocol_limit() {
    // 8 MiB interim cap (#1094); derives from the shared line-io constant.
    assert_eq!(MAX_LINE_BYTES, 8 * 1_048_576);
}

// ── #1061 lockstep: paged history cursor reaches the wire ────────────────

#[test]
fn control_commands_serialize_to_wire() {
    let steer = command_to_json(
        AgentCommand::Steer {
            message: "go".into(),
        },
        "s1",
    );
    assert_eq!(steer["type"], "steer");
    assert_eq!(steer["message"], "go");
    assert_eq!(steer["id"], "s1");

    let follow = command_to_json(
        AgentCommand::FollowUp {
            message: "later".into(),
        },
        "f1",
    );
    assert_eq!(follow["type"], "follow_up");
    assert_eq!(follow["message"], "later");

    assert_eq!(
        command_to_json(AgentCommand::GetSubagents, "g1")["type"],
        "get_subagents"
    );
    assert_eq!(
        command_to_json(AgentCommand::GetToolCatalogue, "g2")["type"],
        "get_tool_catalogue"
    );
    let policy = command_to_json(
        AgentCommand::SetToolPolicy {
            mutations: vec![crate::application::ports::agent_gateway::ToolPolicyMutationPayload {
                tool_id: None,
                name: Some("alpha".into()),
                scope: crate::application::ports::agent_gateway::ToolPolicyScopePayload::Child,
                reason: Some("test".into()),
            }],
            mode: crate::application::ports::agent_gateway::ToolPolicyApplyModePayload::AtNextTurnBoundary,
            operation: crate::application::ports::agent_gateway::ToolPolicyOperationPayload::Patch,
            unlisted_scope: None,
        },
        "p1",
    );
    assert_eq!(policy["type"], "set_tool_policy");
    assert_eq!(policy["mode"], "atNextTurnBoundary");
    assert_eq!(policy["mutations"][0]["scope"], "child");
    let effort = command_to_json(
        AgentCommand::SetEffort {
            effort: "high".into(),
        },
        "e1",
    );
    assert_eq!(effort["type"], "set_effort");
    assert_eq!(effort["effort"], "high");
    assert_eq!(effort["id"], "e1");
}

#[test]
fn get_messages_command_serializes_optional_before_cursor() {
    let newest = command_to_json(AgentCommand::GetMessages { before: None }, "req1");
    assert_eq!(newest["type"], "get_messages");
    assert!(
        newest.get("before").is_none(),
        "no cursor field when unset: {newest}"
    );

    let older = command_to_json(
        AgentCommand::GetMessages {
            before: Some("cursor-id".into()),
        },
        "req2",
    );
    assert_eq!(older["before"], "cursor-id");
}

// ── #1060 lockstep: refs preserved through the API event model ──────────

#[test]
fn get_message_command_serializes_to_wire() {
    let v = command_to_json(
        AgentCommand::GetMessage {
            message_id: "m1".into(),
            agent_id: None,
            tool_call_id: None,
            offset: None,
            limit: None,
        },
        "req1",
    );
    assert_eq!(v["type"], "get_message");
    assert_eq!(v["messageId"], "m1");
    assert_eq!(v["id"], "req1");
    assert!(v.get("agent_id").is_none());
    assert!(v.get("offset").is_none());
    assert!(v.get("limit").is_none());

    let child = command_to_json(
        AgentCommand::GetMessage {
            message_id: "m2".into(),
            agent_id: Some("worker".into()),
            tool_call_id: None,
            offset: Some(4096),
            limit: Some(8192),
        },
        "req2",
    );
    assert_eq!(child["agent_id"], "worker");
    assert_eq!(child["offset"], 4096);
    assert_eq!(child["limit"], 8192);
}

#[test]
fn agent_end_preserves_message_refs_round_trip() {
    // The harness emits agent_end refs-based (empty messages) after #1060.
    let wire = r#"{"type":"agent_end","messages":[],"messageRefs":["a","b"]}"#;
    let ev: AgentEvent = serde_json::from_str(wire).expect("parse agent_end");
    match &ev {
        AgentEvent::AgentEnd { message_refs, .. } => {
            assert_eq!(message_refs, &vec!["a".to_string(), "b".to_string()]);
        }
        other => panic!("expected AgentEnd, got {other:?}"),
    }
    // Re-serialized to a WS client, the refs must survive (not be dropped).
    let out = serde_json::to_value(&ev).unwrap();
    assert_eq!(out["messageRefs"], serde_json::json!(["a", "b"]));
}

#[test]
fn tool_catalogue_changed_is_modeled_not_unknown() {
    let wire = r#"{"type":"tool_catalogue_changed","changedTools":["weather"],"before":[],"after":[{"name":"weather"}],"reason":"register_tool"}"#;
    let ev: AgentEvent = serde_json::from_str(wire).expect("parse");
    match &ev {
        AgentEvent::ToolCatalogueChanged {
            changed_tools,
            before,
            after,
            reason,
        } => {
            assert_eq!(changed_tools, &vec!["weather".to_string()]);
            assert!(before.is_empty());
            assert_eq!(after[0]["name"], "weather");
            assert_eq!(reason, "register_tool");
        }
        other => panic!("expected ToolCatalogueChanged, not Unknown; got {other:?}"),
    }
    let out = serde_json::to_value(&ev).unwrap();
    assert_eq!(out["type"], "tool_catalogue_changed");
    assert_eq!(out["changedTools"], serde_json::json!(["weather"]));
}

#[test]
fn subagent_messages_appended_is_modeled_not_unknown() {
    let wire = r#"{"type":"subagent_messages_appended","agent_id":"worker","messages":[],"messageRefs":["x"]}"#;
    let ev: AgentEvent = serde_json::from_str(wire).expect("parse");
    match &ev {
        AgentEvent::SubagentMessagesAppended {
            agent_id,
            message_refs,
            ..
        } => {
            assert_eq!(agent_id, "worker");
            assert_eq!(message_refs, &vec!["x".to_string()]);
        }
        other => panic!("expected SubagentMessagesAppended, not Unknown; got {other:?}"),
    }
    let out = serde_json::to_value(&ev).unwrap();
    assert_eq!(out["type"], "subagent_messages_appended");
    assert_eq!(out["messageRefs"], serde_json::json!(["x"]));
}

// ── Socket-driven gateway behaviour ──────────────────────────────────────
//
// These stand up a real UnixListener acting as a stub agent so the
// send/enqueue/subscribe correlation logic is exercised end-to-end.

use tokio::io::{AsyncBufReadExt, BufReader as TokioBufReader};

/// Spawn a stub agent that reads one NDJSON command line, echoes back a
/// correlated `Response` carrying the same `id`, and (optionally) applies a
/// transform to the reply. Returns the socket path (kept alive by `dir`).
async fn spawn_echo_agent(
    dir: &tempfile::TempDir,
    reply_command: &'static str,
) -> std::path::PathBuf {
    let socket_path = dir.path().join("agent.sock");
    let listener = tokio::net::UnixListener::bind(&socket_path).expect("bind");
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut lines = TokioBufReader::new(read_half).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let cmd: serde_json::Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let id = cmd["id"].as_str().unwrap_or_default().to_string();
            let reply = serde_json::json!({
                "type": "response",
                "id": id,
                "command": reply_command,
                "success": true,
                "data": {"echo": true},
            });
            let mut bytes = serde_json::to_vec(&reply).unwrap();
            bytes.push(b'\n');
            write_half.write_all(&bytes).await.expect("write reply");
        }
    });
    socket_path
}

#[tokio::test]
async fn send_returns_correlated_response() {
    let dir = tempfile::tempdir().unwrap();
    let path = spawn_echo_agent(&dir, "get_state").await;
    let gw = UdsGateway::connect(&path).await.unwrap();

    let event = gw.send(AgentCommand::GetState).await.unwrap();
    match event {
        AgentEvent::Response {
            command, success, ..
        } => {
            assert_eq!(command, "get_state");
            assert!(success);
        }
        other => panic!("expected Response, got {other:?}"),
    }
}

/// Spawn a stub agent that ignores the request's correlation id and replies
/// with an uncorrelated `agent_error` response. Exercises `send()`'s fallback
/// that matches `agent_error` to an in-flight `prompt` when the id is absent.
async fn spawn_agent_error_agent(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let socket_path = dir.path().join("agent.sock");
    let listener = tokio::net::UnixListener::bind(&socket_path).expect("bind");
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut lines = TokioBufReader::new(read_half).lines();
        while let Ok(Some(_line)) = lines.next_line().await {
            // Deliberately omit the correlation id so only the command-name
            // fallback can match this response.
            let reply = serde_json::json!({
                "type": "response",
                "command": "agent_error",
                "success": false,
                "error": "boom",
            });
            let mut bytes = serde_json::to_vec(&reply).unwrap();
            bytes.push(b'\n');
            write_half.write_all(&bytes).await.expect("write reply");
        }
    });
    socket_path
}

#[tokio::test]
async fn send_matches_uncorrelated_agent_error_for_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let path = spawn_agent_error_agent(&dir).await;
    let gw = UdsGateway::connect(&path).await.unwrap();

    let event = gw
        .send(AgentCommand::Prompt {
            message: "hi".into(),
            streaming_behavior: None,
        })
        .await
        .unwrap();
    match event {
        AgentEvent::Response {
            command, success, ..
        } => {
            assert_eq!(command, "agent_error");
            assert!(!success);
        }
        other => panic!("expected agent_error Response, got {other:?}"),
    }
}

#[tokio::test]
async fn enqueue_returns_accepted_without_waiting() {
    let dir = tempfile::tempdir().unwrap();
    let path = spawn_echo_agent(&dir, "prompt").await;
    let gw = UdsGateway::connect(&path).await.unwrap();

    let event = gw
        .enqueue(AgentCommand::Prompt {
            message: "hi".into(),
            streaming_behavior: None,
        })
        .await
        .unwrap();
    match event {
        AgentEvent::Response {
            command,
            success,
            data,
            ..
        } => {
            assert_eq!(command, "prompt");
            assert!(success);
            assert_eq!(data.unwrap()["accepted"], true);
        }
        other => panic!("expected accepted Response, got {other:?}"),
    }
}

#[tokio::test]
async fn subscribe_receives_broadcast_events() {
    let dir = tempfile::tempdir().unwrap();
    let socket_path = dir.path().join("agent.sock");
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
    let (subscribed_tx, subscribed_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        subscribed_rx.await.ok();
        stream
            .write_all(b"{\"type\":\"token\",\"token\":\"hello\"}\n")
            .await
            .unwrap();
    });

    let gw = UdsGateway::connect(&socket_path).await.unwrap();
    let mut sub = gw.subscribe().await.unwrap();
    subscribed_tx.send(()).unwrap();

    let event = tokio::time::timeout(std::time::Duration::from_secs(3), sub.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(event, AgentEvent::Token { token } if token == "hello"));
}

#[tokio::test]
async fn send_fails_when_disconnected() {
    let dir = tempfile::tempdir().unwrap();
    let socket_path = dir.path().join("agent.sock");
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
    // Accept then immediately drop the connection so the reader marks the
    // gateway disconnected.
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        drop(stream);
    });
    let gw = UdsGateway::connect(&socket_path).await.unwrap();

    // Wait for the reader task to observe EOF and flip `connected`.
    for _ in 0..100 {
        if !gw.is_connected() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(!gw.is_connected());
    let err = gw.send(AgentCommand::GetState).await.unwrap_err();
    assert!(matches!(err, ApiError::AgentNotConnected));
    let err = gw.enqueue(AgentCommand::GetState).await.unwrap_err();
    assert!(matches!(err, ApiError::AgentNotConnected));
}

#[tokio::test]
async fn connect_fails_for_missing_socket() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope.sock");
    let err = UdsGateway::connect(&missing).await.unwrap_err();
    assert!(matches!(err, ApiError::Internal(_)));
}

#[tokio::test]
async fn debug_impl_reports_connection_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = spawn_echo_agent(&dir, "get_state").await;
    let gw = UdsGateway::connect(&path).await.unwrap();
    assert!(format!("{gw:?}").contains("connected"));
}

#[test]
fn prompt_command_serializes_streaming_behavior() {
    let with_sb = command_to_json(
        AgentCommand::Prompt {
            message: "hi".into(),
            streaming_behavior: Some("steer".into()),
        },
        "p1",
    );
    assert_eq!(with_sb["type"], "prompt");
    assert_eq!(with_sb["message"], "hi");
    assert_eq!(with_sb["streamingBehavior"], "steer");

    let without = command_to_json(
        AgentCommand::Prompt {
            message: "hi".into(),
            streaming_behavior: None,
        },
        "p2",
    );
    assert!(without.get("streamingBehavior").is_none());
}

#[test]
fn remaining_commands_serialize_to_wire() {
    assert_eq!(command_to_json(AgentCommand::Abort, "a")["type"], "abort");
    assert_eq!(
        command_to_json(AgentCommand::GetSessionStats, "s")["type"],
        "get_session_stats"
    );
    assert_eq!(
        command_to_json(AgentCommand::ClearHistory, "c")["type"],
        "clear_history"
    );
    let tail = command_to_json(AgentCommand::GetMessagesTail { count: 7 }, "t");
    assert_eq!(tail["type"], "get_messages_tail");
    assert_eq!(tail["count"], 7);

    let model = command_to_json(
        AgentCommand::SetModel {
            model: None,
            provider: Some("openai".into()),
            model_id: Some("gpt".into()),
        },
        "m",
    );
    assert_eq!(model["provider"], "openai");
    assert_eq!(model["modelId"], "gpt");
    assert!(model.get("model").is_none());
}

#[test]
fn tool_policy_changed_is_modeled_not_unknown() {
    let wire = r#"{"type":"tool_policy_changed","changedTools":["alpha"],"results":[{"name":"alpha","before":{"effectiveScope":"both"},"after":{"effectiveScope":"child"}}],"applyMode":"immediateIfIdle","reason":"set_tool_policy","correlationId":"req-1"}"#;
    let ev: AgentEvent = serde_json::from_str(wire).expect("parse");
    match &ev {
        AgentEvent::ToolPolicyChanged {
            changed_tools,
            results,
            apply_mode,
            reason,
            correlation_id,
        } => {
            assert_eq!(changed_tools, &vec!["alpha".to_string()]);
            assert_eq!(results[0]["after"]["effectiveScope"], "child");
            assert_eq!(apply_mode, "immediateIfIdle");
            assert_eq!(reason, "set_tool_policy");
            assert_eq!(correlation_id.as_deref(), Some("req-1"));
        }
        other => panic!("expected ToolPolicyChanged, got {other:?}"),
    }
}
