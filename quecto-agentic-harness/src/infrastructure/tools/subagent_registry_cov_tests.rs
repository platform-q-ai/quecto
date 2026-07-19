use super::*;

fn poison_registry(registry: &SubagentRegistry) {
    let cloned = registry.clone();
    let _ = std::thread::spawn(move || {
        let _guard = cloned.lock().unwrap();
        panic!("poison registry for coverage");
    })
    .join();
    assert!(registry.lock().is_err(), "registry should be poisoned");
}

#[test]
fn status_wire_strings_round_trip_and_unknown_defaults_to_starting() {
    for (status, wire, display) in [
        (SubagentStatus::Starting, "starting", "Starting"),
        (SubagentStatus::Idle, "idle", "Idle"),
        (SubagentStatus::Running, "running", "Running"),
        (SubagentStatus::Error, "error", "Error"),
        (SubagentStatus::Exited, "exited", "Exited"),
    ] {
        assert_eq!(status.to_wire_str(), wire);
        assert_eq!(SubagentStatus::from_wire_str(wire), status);
        assert_eq!(status.to_string(), display);
    }
    assert_eq!(
        SubagentStatus::from_wire_str("paused"),
        SubagentStatus::Starting
    );
}

#[test]
fn new_entry_sets_latches_and_lookup_reports_registered_socket() {
    let registry = new_registry();
    let socket = std::path::PathBuf::from("/tmp/child.sock");
    registry
        .lock()
        .unwrap()
        .insert("child".into(), SubagentEntry::new(socket.clone(), 1234));
    assert_eq!(lookup_subagent_socket(&registry, "child").unwrap(), socket);
    let entries = registry.lock().unwrap();
    let entry = entries.get("child").unwrap();
    assert_eq!(entry.pid, 1234);
    assert_eq!(entry.status, SubagentStatus::Starting);
    assert!(entry.completion_armed);
    assert!(entry.stalled_armed);
    assert!(!entry.read_only);
}

#[test]
fn completion_dedupe_helpers_consume_once_and_handle_missing_registry() {
    let registry = new_registry();
    registry
        .lock()
        .unwrap()
        .insert("child".into(), SubagentEntry::new("/tmp/sock".into(), 1));
    assert!(!take_completion_consumed_by_await(&registry, "child"));
    mark_completion_consumed_by_await(&registry, "child");
    assert!(consume_await_dedupe(&Some(registry.clone()), "child"));
    assert!(!consume_await_dedupe(&Some(registry), "child"));
    assert!(!consume_await_dedupe(&None, "child"));
}

#[test]
fn sequenced_notification_keys_messages_and_completion_flag_are_variant_specific() {
    let completed = SequencedSubagentNotification::new(
        9,
        SubagentNotification::Completed {
            agent_id: "a".into(),
        },
    );
    assert_eq!(completed.dedupe_key(), ("a".to_string(), 9));
    assert!(completed.is_completion());
    assert!(completed.to_message().contains("ended a turn"));
    let stalled = SequencedSubagentNotification::new(
        10,
        SubagentNotification::Stalled {
            agent_id: "b".into(),
            workflow_mode: "active".into(),
            steps_completed: 1,
            steps_total: 3,
        },
    );
    assert_eq!(stalled.dedupe_key(), ("b".to_string(), 10));
    assert!(!stalled.is_completion());
    assert!(stalled.to_message().contains("1/3"));
    let errored = SequencedSubagentNotification::new(
        11,
        SubagentNotification::Errored {
            agent_id: "c".into(),
            error: "boom".into(),
        },
    );
    assert_eq!(errored.dedupe_key(), ("c".to_string(), 11));
    assert!(errored.to_message().contains("boom"));
    let exited = SequencedSubagentNotification::new(
        12,
        SubagentNotification::Exited {
            agent_id: "d".into(),
        },
    );
    assert_eq!(exited.dedupe_key(), ("d".to_string(), 12));
    assert!(exited.to_message().contains("exited unexpectedly"));
}

#[test]
fn missing_lookup_and_dedupe_helpers_are_safe() {
    let registry = new_registry();
    let err = lookup_subagent_socket(&registry, "ghost").unwrap_err();
    assert!(err.contains("ghost"), "{err}");
    mark_completion_consumed_by_await(&registry, "ghost");
    assert!(!take_completion_consumed_by_await(&registry, "ghost"));
    assert!(registry.lock().unwrap().is_empty());
}

#[test]
fn registry_lock_poison_recovery_helpers_still_work() {
    let registry = new_registry();
    registry.lock().unwrap().insert(
        "child".into(),
        SubagentEntry::new(std::path::PathBuf::from("/tmp/poison.sock"), 42),
    );
    poison_registry(&registry);

    assert_eq!(
        lookup_subagent_socket(&registry, "child").unwrap(),
        std::path::PathBuf::from("/tmp/poison.sock")
    );
    mark_completion_consumed_by_await(&registry, "child");
    assert!(take_completion_consumed_by_await(&registry, "child"));
    assert!(!consume_await_dedupe(&Some(registry.clone()), "child"));
    assert!(
        registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key("child")
    );
}

#[tokio::test]
async fn send_subagent_command_connect_timeout_and_closed_errors() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.sock");
    let err = send_subagent_uds_command_with_timeout(
        &missing,
        r#"{"type":"get_state"}"#,
        std::time::Duration::from_millis(20),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("connect to subagent"), "{err}");

    let timeout_sock = dir.path().join("timeout.sock");
    let listener = tokio::net::UnixListener::bind(&timeout_sock).unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = tokio::io::BufReader::new(stream);
        let _ = crate::infrastructure::test_support::read_framed_command_async(&mut reader).await;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    });
    let err = send_subagent_uds_command_with_timeout(
        &timeout_sock,
        r#"{"type":"get_state"}"#,
        std::time::Duration::from_millis(20),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("timed out"), "{err}");
    server.abort();

    let closed_sock = dir.path().join("closed.sock");
    let listener = tokio::net::UnixListener::bind(&closed_sock).unwrap();
    let server = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
    });
    let err = send_subagent_uds_command_with_timeout(
        &closed_sock,
        r#"{"type":"get_state"}"#,
        std::time::Duration::from_millis(200),
    )
    .await
    .unwrap_err();
    assert!(
        err.to_string().contains("closed connection")
            || err.to_string().contains("read from subagent failed"),
        "{err}"
    );
    server.await.unwrap();
}

#[tokio::test]
async fn send_subagent_command_non_object_accepts_first_response() {
    use tokio::io::AsyncWriteExt;
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("plain.sock");
    let listener = tokio::net::UnixListener::bind(&sock).unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        stream
            .write_all(
                br#"{"type":"response","success":true,"data":{"ok":true}}
"#,
            )
            .await
            .unwrap();
    });

    let response = send_subagent_uds_command_with_timeout(
        &sock,
        "not-json",
        std::time::Duration::from_secs(1),
    )
    .await
    .unwrap();
    assert!(response.contains(r#""ok":true"#), "{response}");
    server.await.unwrap();
}
