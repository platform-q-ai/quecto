use super::container_script_cleanup::cleanup_container_environments_after_removal;
use super::spawn::rollback_existing_join;
use super::subagent_registry::{ExitSignal, SubagentRegistry};
use crate::domain::ids::AgentUuid;

pub(super) async fn rollback_registered_spawn_failure(
    registry: &SubagentRegistry,
    registry_key: &str,
    broadcast_tx: Option<&tokio::sync::broadcast::Sender<String>>,
    container_registry: &crate::infrastructure::tools::container_registry::ContainerRegistry,
    existing_join: Option<&super::spawn_container_existing::ExistingContainerJoin>,
    agent_uuid: &AgentUuid,
) -> Result<(), String> {
    let root_exit_rx = registry
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(registry_key)
        .and_then(|entry| entry.exit_signal_tx.as_ref().map(|tx| tx.subscribe()));

    let crate::infrastructure::tools::subagent_cascade::CascadeOutcome { removed, event } =
        crate::infrastructure::tools::subagent_cascade::cascade_remove_and_state_changed(
            registry,
            registry_key,
        );
    if let Some(event) = event {
        if let Some(tx) = broadcast_tx {
            let _ = tx.send(event);
        }
    }

    for (id, entry) in &removed {
        if let Some(ref tx) = entry.exit_signal_tx {
            let _ = tx.send(Some(ExitSignal {
                exit_code: None,
                signal: Some(15),
            }));
        }
        crate::infrastructure::tools::subagent_cascade::terminate_removed_entry(entry);
        tracing::debug!(agent = %id, "rolled back registered spawn after launch failure");
    }

    let cleanup_result = cleanup_container_environments_after_removal(&removed, registry);
    rollback_existing_join(container_registry, existing_join, agent_uuid);

    if let Some(mut rx) = root_exit_rx {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), async move {
            loop {
                if rx.borrow().is_some() {
                    break;
                }
                if rx.changed().await.is_err() {
                    break;
                }
            }
        })
        .await;
    }
    cleanup_result
}
