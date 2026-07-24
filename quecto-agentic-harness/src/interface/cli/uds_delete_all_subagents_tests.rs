use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::infrastructure::tools::subagent_registry::{SubagentEntry, SubagentRegistry};

#[test]
fn delete_all_subagents_clears_registry() {
    let registry: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    registry.lock().unwrap().insert(
        "worker".into(),
        SubagentEntry::new(PathBuf::from("/tmp/worker.sock"), 0),
    );
    registry.lock().unwrap().insert(
        "reviewer".into(),
        SubagentEntry::new(PathBuf::from("/tmp/reviewer.sock"), 0),
    );

    let removed = super::delete_all_subagents_from_registry(&registry, None);

    assert_eq!(removed, 2);
    assert!(
        registry.lock().unwrap().is_empty(),
        "delete-all-subagents must remove every entry from the harness registry"
    );
}

#[test]
fn delete_all_subagents_signals_awaiters_before_registry_cleanup() {
    let registry: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/worker.sock"), 0);
    let (tx, mut rx) = tokio::sync::watch::channel(None);
    entry.exit_signal_tx = Some(tx);
    registry.lock().unwrap().insert("worker".into(), entry);

    let removed = super::delete_all_subagents_from_registry(&registry, None);

    assert_eq!(removed, 1);
    let signal = rx
        .borrow_and_update()
        .clone()
        .expect("delete-all-subagents should notify awaiters before removing registry entries");
    assert_eq!(signal.exit_code, None);
    assert_eq!(signal.signal, Some(15));
    assert!(registry.lock().unwrap().is_empty());
}

#[tokio::test]
async fn delete_all_subagents_broadcasts_empty_registry_snapshot() {
    let registry: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    registry.lock().unwrap().insert(
        "worker".into(),
        SubagentEntry::new(PathBuf::from("/tmp/worker.sock"), 0),
    );
    let (tx, mut rx) = tokio::sync::broadcast::channel(4);

    let removed = super::delete_all_subagents_from_registry(&registry, Some(&tx));

    assert_eq!(removed, 1);
    let event = rx.recv().await.unwrap();
    let value: serde_json::Value = serde_json::from_str(event.trim()).unwrap();
    assert_eq!(value["type"], "subagent_state_changed");
    assert_eq!(value["subagents"].as_array().unwrap().len(), 0);
    assert!(registry.lock().unwrap().is_empty());
}
