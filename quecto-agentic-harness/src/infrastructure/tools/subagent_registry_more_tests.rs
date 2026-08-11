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
