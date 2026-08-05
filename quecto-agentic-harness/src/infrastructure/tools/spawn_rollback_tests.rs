use super::container_registry::new_container_registry;
use super::spawn::SpawnTool;
use super::spawn_entry::{InitialRegistryEntrySpec, child_session_key, initial_registry_entry};
use super::spawn_registry::register_and_broadcast;
use super::subagent_registry::new_exit_signal_channel;
use crate::domain::error::DomainError;
use crate::domain::subagent::SubagentConfig;
use std::path::PathBuf;

#[tokio::test]
async fn rollback_registered_spawn_failure_removes_entry_and_preserves_error() {
    let tool = SpawnTool::new(vec![], true);
    let uuid = crate::domain::ids::AgentUuid::new("aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee");
    let key = child_session_key(&uuid);
    let (exit_tx, exit_rx) = new_exit_signal_channel();
    let mut entry = initial_registry_entry(InitialRegistryEntrySpec {
        agent_uuid: uuid.clone(),
        display_name: "faulty".into(),
        socket_path: PathBuf::from("/tmp/missing.sock"),
        pid: 0,
        parent_id: None,
        config: &SubagentConfig {
            task: Some("boom".into()),
            agent_id: Some("faulty".into()),
            restrict_to_workspace: true,
            system: None,
            config_path: None,
            workflow: false,
            workflow_guards: false,
            workflow_spec: None,
            model: None,
            effort: None,
            disable_tools: vec![],
            read_only: false,
            container: crate::domain::container_runtime::SpawnContainerRequest::Local,
        },
        exit_signal_tx: Some(exit_tx),
    });
    let monitor = tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    });
    let monitor_abort = monitor.abort_handle();
    entry.monitor_handle = Some(std::sync::Arc::new(monitor));
    let registry = tool.registry();
    let container_registry = new_container_registry();
    register_and_broadcast(registry, None, "faulty", entry);

    let err =
        DomainError::Tool("failed to send prompt to subagent: injected prompt failure".into());
    super::spawn_rollback::rollback_registered_spawn_failure(
        registry,
        key,
        None,
        &container_registry,
        None,
        &uuid,
    )
    .await;

    assert!(
        registry.lock().unwrap().get(key).is_none(),
        "registry entry removed"
    );
    tokio::task::yield_now().await;
    assert!(
        monitor_abort.is_finished(),
        "monitor aborted during rollback"
    );
    assert!(
        exit_rx.borrow().is_some(),
        "exit/await resources signalled and cleaned"
    );
    assert_eq!(
        err.to_string(),
        "tool error: failed to send prompt to subagent: injected prompt failure"
    );

    super::spawn_rollback::rollback_registered_spawn_failure(
        registry,
        key,
        None,
        &container_registry,
        None,
        &uuid,
    )
    .await;
    assert!(
        registry.lock().unwrap().is_empty(),
        "rollback is idempotent"
    );
}
