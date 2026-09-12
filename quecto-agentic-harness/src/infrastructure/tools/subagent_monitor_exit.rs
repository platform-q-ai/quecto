//! Terminal handling of one monitored child (#1369 slice 3): the monitor
//! connection's EOF/reset or connect failure is the death signal for a
//! script-managed child, and every child's exit drives the same
//! exactly-once cleanup, cascade removal, exit signal and passive note.
use super::{
    NotificationTx, SubagentNotification, SubagentRegistry, mark_exited, send_notification,
    update_entry_next_sequence,
};

pub(super) async fn notify_child_exited(
    registry: &SubagentRegistry,
    agent_id: &str,
    notify_tx: Option<&NotificationTx>,
    broadcast_tx: Option<&tokio::sync::broadcast::Sender<String>>,
    kind: super::super::subagent_registry::ExitSignalKind,
) {
    // #1369 slice 3: a script-managed child has no local process to reap, so
    // the monitor connection's EOF/reset IS its death signal. Feed the
    // existing exit signal so lifecycle observers wake instantly; the local-child
    // reaper keeps owning the signal (with the real exit status) when a
    // process exists.
    let exit_tx = {
        let entries = registry.lock().unwrap_or_else(|e| e.into_inner());
        entries
            .get(agent_id)
            .filter(|entry| entry.pid == 0)
            .and_then(|entry| entry.exit_signal_tx.clone())
    };
    // Post-mortem inspect + membership/cleanup claims run BEFORE the exit
    // signal fires and BEFORE the entry is marked exited: a woken `await`er
    // (or its immediate `get_containers`) must observe the authoritative
    // aggregate already updated, per the documented contract.
    super::super::subagent_cleanup::cleanup_registered_once(registry, agent_id).await;
    let sequence = update_entry_next_sequence(registry, agent_id, mark_exited);
    // Terminal transition: nothing may connect to a dead child's bridge, so
    // tear the accept loop and its socket file down while the entry itself
    // stays listed as exited until the cascade prune below.
    {
        let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = entries.get_mut(agent_id) {
            super::super::spawn_proxy_bridge::teardown_entry_bridge(
                entry.proxy_bridge_handle.take().as_ref(),
                entry.proxy_bridge_socket.take().as_deref(),
            );
        }
    }
    let label = notification_display_label(registry, agent_id);
    let agent_uuid = notification_agent_uuid(registry, agent_id);
    let super::super::subagent_cascade::CascadeOutcome { removed, event } =
        super::super::subagent_cascade::cascade_remove_and_state_changed(registry, agent_id);
    if let Some(event) = event {
        if let Some(tx) = broadcast_tx {
            // Push the survivor-only roster onto the live event stream (snapshots
            // and the TUI roster observe the dead subtree removal without polling).
            let _ = tx.send(event);
        }
    }
    let mut removed = removed;
    super::super::subagent_cleanup::cleanup_removed_entries_once(
        &mut removed,
        super::super::subagent_cleanup::FinalizeMode::Exit,
    )
    .await;
    for (id, entry) in &removed {
        if id == agent_id {
            continue;
        }
        if let Some(ref tx) = entry.exit_signal_tx {
            tx.send_replace(Some(super::super::subagent_registry::ExitSignal {
                exit_code: None,
                signal: None,
                kind,
            }));
        }
        super::super::subagent_cascade::terminate_removed_entry(entry);
    }
    if let Some(tx) = exit_tx {
        // No exit status exists for a script-managed death: the kind keeps
        // the await reason honest (connection_closed / never_reachable)
        // instead of fabricating a clean exit. Publish only after cascade
        // cleanup/broadcast and descendant signals so a woken awaiter observes
        // the authoritative survivor set and terminal subtree.
        tx.send_replace(Some(super::super::subagent_registry::ExitSignal {
            exit_code: None,
            signal: None,
            kind,
        }));
    }
    send_notification(
        notify_tx,
        super::super::subagent_registry::SequencedSubagentNotification::new_for_agent(
            sequence,
            SubagentNotification::Exited {
                agent_id: label,
                reason: Some(kind.to_wire_str().to_string()),
            },
            agent_uuid,
        ),
    );
}

pub(super) fn notification_display_label(registry: &SubagentRegistry, agent_id: &str) -> String {
    let entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    entries
        .get(agent_id)
        .map(|entry| entry.effective_display_name(agent_id).to_string())
        .unwrap_or_else(|| agent_id.to_string())
}

pub(super) fn notification_agent_uuid(
    registry: &SubagentRegistry,
    agent_id: &str,
) -> crate::domain::ids::AgentUuid {
    let entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    entries
        .get(agent_id)
        .map(|entry| entry.agent_uuid.clone())
        .unwrap_or_else(|| crate::domain::ids::AgentUuid::new(agent_id))
}
