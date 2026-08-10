use super::*;
use std::path::PathBuf;

fn test_entry() -> SubagentEntry {
    SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0)
}

#[tokio::test]
async fn notify_child_exited_cascades_descendants_and_reports_reason() {
    let registry = super::super::subagent_registry::new_registry();
    let (notify_tx, mut notify_rx) = super::super::subagent_registry::new_notification_channel();
    let (broadcast_tx, mut broadcast_rx) = tokio::sync::broadcast::channel::<String>(4);
    let mut parent = test_entry();
    parent.display_name = "parent".into();
    let mut child = test_entry();
    child.parent_id = Some("parent".into());
    let (child_exit_tx, mut child_exit_rx) =
        super::super::subagent_registry::new_exit_signal_channel();
    child.exit_signal_tx = Some(child_exit_tx);
    let mut grandchild = test_entry();
    grandchild.parent_id = Some("child".into());
    let (grandchild_exit_tx, mut grandchild_exit_rx) =
        super::super::subagent_registry::new_exit_signal_channel();
    grandchild.exit_signal_tx = Some(grandchild_exit_tx);
    let mut sibling = test_entry();
    sibling.display_name = "sibling".into();
    {
        let mut entries = registry.lock().unwrap();
        entries.insert("parent".into(), parent);
        entries.insert("child".into(), child);
        entries.insert("grandchild".into(), grandchild);
        entries.insert("sibling".into(), sibling);
    }

    notify_child_exited(
        &registry,
        "parent",
        Some(&notify_tx),
        Some(&broadcast_tx),
        super::super::subagent_registry::ExitSignalKind::ConnectionClosed,
    )
    .await;

    let entries = registry.lock().unwrap();
    assert!(!entries.contains_key("parent"));
    assert!(!entries.contains_key("child"));
    assert!(!entries.contains_key("grandchild"));
    assert!(entries.contains_key("sibling"));
    drop(entries);
    let event: serde_json::Value = serde_json::from_str(&broadcast_rx.try_recv().unwrap()).unwrap();
    let ids: Vec<_> = event["subagents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["agentId"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["sibling"]);
    let note = notify_rx.try_recv().unwrap();
    assert_eq!(
        note.to_message(),
        "Agent 'parent' exited unexpectedly (connection_closed)"
    );
    for rx in [&mut child_exit_rx, &mut grandchild_exit_rx] {
        let descendant_exit = rx
            .borrow_and_update()
            .clone()
            .expect("descendant exit signal published");
        assert_eq!(
            descendant_exit.kind,
            super::super::subagent_registry::ExitSignalKind::ConnectionClosed
        );
    }
}
