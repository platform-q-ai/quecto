use super::*;

fn test_entry() -> SubagentEntry {
    SubagentEntry::new(std::path::PathBuf::from("/tmp/test.sock"), 1234)
}

#[tokio::test]
async fn monitor_connect_failure_marks_socket_readiness_failure() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("missing.sock");
    let registry = super::super::subagent_registry::new_registry();
    registry
        .lock()
        .unwrap()
        .insert("child".to_string(), test_entry());
    let (tx, mut rx) = super::super::subagent_registry::new_notification_channel();

    let handle = spawn_monitor_task(
        "child".to_string(),
        sock,
        registry.clone(),
        Some(tx),
        None,
        None,
    );

    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if registry.lock().unwrap()["child"].status == SubagentStatus::Exited {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("monitor should give up connecting to the missing socket");

    let entry = registry.lock().unwrap().get("child").unwrap().clone();
    assert_eq!(
        entry.lifecycle,
        super::super::subagent_lifecycle::SubagentLifecycleState::Exited
    );
    assert_eq!(entry.status, SubagentStatus::Exited);
    assert!(
        rx.try_recv().is_ok(),
        "connect failure should notify parent"
    );
    handle.abort();
}
