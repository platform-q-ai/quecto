use super::*;

#[test]
fn exit_signal_kind_wire_strings_cover_all_variants() {
    assert_eq!(ExitSignalKind::ProcessExit.to_wire_str(), "process_exit");
    assert_eq!(
        ExitSignalKind::ConnectionClosed.to_wire_str(),
        "connection_closed"
    );
    assert_eq!(
        ExitSignalKind::NeverReachable.to_wire_str(),
        "never_reachable"
    );
}

#[test]
fn sequenced_notification_helpers_cover_identity_dedupe_and_messages() {
    let completed = SequencedSubagentNotification::new(
        7,
        SubagentNotification::Completed {
            agent_id: "bot".into(),
        },
    );
    assert_eq!(completed.dedupe_key(), ("bot".to_string(), 7));
    assert!(completed.is_completion());
    assert!(completed.to_message().contains("ended a turn"));
    assert!(completed.agent_uuid.is_none());

    let uuid = AgentUuid::from("uuid-1".to_string());
    let errored = SequencedSubagentNotification::new_for_agent(
        8,
        SubagentNotification::Errored {
            agent_id: "bot".into(),
            error: "boom".into(),
        },
        uuid.clone(),
    );
    assert_eq!(errored.agent_uuid, Some(uuid));
    assert_eq!(errored.dedupe_key(), ("bot".to_string(), 8));
    assert!(!errored.is_completion());
    assert!(errored.to_message().contains("boom"));
}

#[test]
fn notification_messages_cover_stalled_and_exited_reason_branches() {
    let stalled = SubagentNotification::Stalled {
        agent_id: "bot".into(),
        workflow_mode: "active".into(),
        steps_completed: 2,
        steps_total: 5,
    };
    assert!(stalled.to_message().contains("2/5"));

    let exited_reason = SubagentNotification::Exited {
        agent_id: "bot".into(),
        reason: Some("signal".into()),
    };
    assert!(exited_reason.to_message().contains("signal"));

    let exited_empty = SubagentNotification::Exited {
        agent_id: "bot".into(),
        reason: Some(String::new()),
    };
    assert_eq!(
        exited_empty.to_message(),
        "Agent 'bot' exited unexpectedly".to_string()
    );
}

#[test]
fn seed_bound_workflow_none_leaves_entry_without_snapshot() {
    let mut entry = SubagentEntry::new("/tmp/no-workflow.sock".into(), 1);
    seed_bound_workflow(&mut entry, None);
    assert!(entry.workflow.is_none());
}

#[test]
fn registry_channel_constructors_start_empty_and_accept_messages() {
    let (exit_tx, exit_rx) = new_exit_signal_channel();
    assert!(exit_rx.borrow().is_none());
    exit_tx
        .send(Some(ExitSignal {
            exit_code: Some(0),
            signal: None,
            kind: ExitSignalKind::ProcessExit,
        }))
        .unwrap();
    assert_eq!(
        exit_rx.borrow().as_ref().unwrap().kind,
        ExitSignalKind::ProcessExit
    );

    let (notify_tx, mut notify_rx) = new_notification_channel();
    notify_tx
        .try_send(SequencedSubagentNotification::new(
            1,
            SubagentNotification::Completed {
                agent_id: "bot".into(),
            },
        ))
        .unwrap();
    assert_eq!(
        notify_rx.try_recv().unwrap().dedupe_key(),
        ("bot".to_string(), 1)
    );
}

#[test]
fn effective_display_name_uses_display_only_for_uuid_keyed_entries() {
    let mut entry = SubagentEntry::with_identity(
        AgentUuid::from("uuid".to_string()),
        "friendly".to_string(),
        "/tmp/friendly.sock".into(),
        1,
    );
    assert_eq!(entry.effective_display_name("uuid"), "friendly");
    assert_eq!(entry.effective_display_name("legacy-key"), "legacy-key");
    entry.display_name.clear();
    assert_eq!(entry.effective_display_name("uuid"), "uuid");
}

#[test]
fn active_descendant_for_agent_covers_missing_parent_fallback_and_none_registry() {
    assert!(!has_active_descendant_for_agent(&None, "parent"));

    let reg = new_registry();
    {
        let mut entries = reg.lock().unwrap();
        let mut child = SubagentEntry::new("/tmp/child.sock".into(), 1);
        child.status = SubagentStatus::Idle;
        child.parent_id = Some("parent".to_string());
        entries.insert("child".to_string(), child);
    }
    assert!(!has_active_descendant_for_agent(
        &Some(reg.clone()),
        "parent"
    ));

    reg.lock().unwrap().get_mut("child").unwrap().status = SubagentStatus::Running;
    assert!(has_active_descendant_for_agent(&Some(reg), "parent"));
}

#[test]
fn validate_agent_id_format_covers_valid_length_and_character_errors() {
    assert!(validate_agent_id_format("agent_1-ok").is_ok());
    assert_eq!(
        validate_agent_id_format("").unwrap_err(),
        "agent_id must be 1-64 characters"
    );
    assert_eq!(
        validate_agent_id_format(&"a".repeat(65)).unwrap_err(),
        "agent_id must be 1-64 characters"
    );
    assert_eq!(
        validate_agent_id_format("bad.name").unwrap_err(),
        "agent_id must use only [a-zA-Z0-9_-]"
    );
}

#[test]
fn exited_notification_without_reason_uses_generic_message() {
    let exited = SubagentNotification::Exited {
        agent_id: "bot".into(),
        reason: None,
    };
    assert_eq!(exited.to_message(), "Agent 'bot' exited unexpectedly");
}

#[test]
fn sequenced_non_completion_notifications_are_not_completion() {
    let exited = SequencedSubagentNotification::new(
        9,
        SubagentNotification::Exited {
            agent_id: "bot".into(),
            reason: None,
        },
    );
    assert!(!exited.is_completion());
    assert_eq!(exited.dedupe_key(), ("bot".to_string(), 9));
}

#[test]
fn lookup_subagent_socket_covers_success_non_live_and_non_connectable() {
    let reg = new_registry();
    {
        let mut entries = reg.lock().unwrap();
        let mut live = SubagentEntry::with_identity(
            AgentUuid::from("uuid-live-socket".to_string()),
            "live".to_string(),
            "/tmp/live.sock".into(),
            1,
        );
        live.status = SubagentStatus::Idle;
        entries.insert("uuid-live-socket".to_string(), live);

        let mut detached = SubagentEntry::with_identity(
            AgentUuid::from("uuid-detached".to_string()),
            "detached".to_string(),
            "/tmp/detached.sock".into(),
            2,
        );
        detached.status = SubagentStatus::Idle;
        detached.persisted_liveness = SubagentLiveness::Detached;
        entries.insert("uuid-detached".to_string(), detached);

        let mut empty_socket = SubagentEntry::with_identity(
            AgentUuid::from("uuid-empty-socket".to_string()),
            "uuid-empty-socket".to_string(),
            std::path::PathBuf::new(),
            3,
        );
        empty_socket.status = SubagentStatus::Idle;
        entries.insert("uuid-empty-socket".to_string(), empty_socket);
    }

    assert_eq!(
        lookup_subagent_socket(&reg, "live").unwrap(),
        std::path::PathBuf::from("/tmp/live.sock")
    );
    let detached_err = lookup_subagent_socket(&reg, "detached").unwrap_err();
    assert!(
        detached_err.contains("not command-targetable")
            || detached_err.contains("no live subagent")
    );
    let empty_err = lookup_subagent_socket(&reg, "uuid-empty-socket").unwrap_err();
    assert!(
        empty_err.contains("no ancestor-connectable socket"),
        "{empty_err}"
    );
}

#[test]
fn registry_lookup_and_request_id_helpers_cover_success_and_fallbacks() {
    let mut entries = std::collections::HashMap::new();
    let mut entry = SubagentEntry::with_identity(
        AgentUuid::from("uuid-live".to_string()),
        "friendly".to_string(),
        "/tmp/friendly.sock".into(),
        42,
    );
    entry.status = SubagentStatus::Idle;
    entries.insert("uuid-live".to_string(), entry);
    assert_eq!(
        resolve_registry_key(&entries, "friendly").unwrap(),
        "uuid-live"
    );
    assert_eq!(
        resolve_registry_key(&entries, "uuid-live").unwrap(),
        "uuid-live"
    );

    let (rewritten, id) = stamp_request_id(r#"{"type":"get_state","id":"old"}"#);
    let id = id.expect("object requests get a correlation id");
    let value: serde_json::Value = serde_json::from_str(&rewritten).unwrap();
    assert_eq!(value["type"], "get_state");
    assert_eq!(value["id"], id);
    assert_ne!(id, "old");

    let (unchanged_array, array_id) = stamp_request_id(r#"["not","an","object"]"#);
    assert_eq!(unchanged_array, r#"["not","an","object"]"#);
    assert!(array_id.is_none());

    let (unchanged_invalid, invalid_id) = stamp_request_id("not json");
    assert_eq!(unchanged_invalid, "not json");
    assert!(invalid_id.is_none());
}

#[tokio::test]
async fn command_reader_returns_busy_get_state_snapshot_without_waiting_for_live_reply() {
    use tokio::io::{AsyncWriteExt, BufReader};
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("busy-state.sock");
    let listener = tokio::net::UnixListener::bind(&sock).unwrap();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = tokio::io::split(stream);
        let snapshot = serde_json::json!({
            "type": "response",
            "command": "get_state",
            "data": {
                "state": "runningTool",
                "effort": null,
                "model": "mock",
                "sessionKey": "cli:dog-story-writer",
                "progress": { "state": "advancing", "reason": "tool activity" },
                "generation": 9
            }
        });
        let mut snapshot_line = snapshot.to_string();
        snapshot_line.push('\n');
        write_half
            .write_all(snapshot_line.as_bytes())
            .await
            .unwrap();

        let mut reader = BufReader::new(read_half);
        let _payload =
            quecto_line_io::read_frame(&mut reader, quecto_line_io::PROTOCOL_FRAME_CAP_BYTES)
                .await
                .expect("parent command should be framed")
                .expect("parent should send get_state");
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    });

    let reply = tokio::time::timeout(
        std::time::Duration::from_millis(250),
        send_subagent_uds_command_with_timeout(
            &sock,
            r#"{"type":"get_state"}"#,
            std::time::Duration::from_secs(10),
        ),
    )
    .await
    .expect("get_state must return the id-less snapshot immediately")
    .expect("snapshot should be accepted");
    let json: serde_json::Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(json["data"]["sessionKey"], "cli:dog-story-writer");
    assert_eq!(json["data"]["generation"], 9);

    server.abort();
}
