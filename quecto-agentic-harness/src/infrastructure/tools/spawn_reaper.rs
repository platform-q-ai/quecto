//! Reaper for locally spawned subagent children.
//!
//! Local launches (only) own an OS child process, held by the one
//! [`OwnedChildSupervisor`] (#1935); this task waits for the supervisor's
//! exit report, translates it into the shared exit signal, and drives the
//! same exactly-once cleanup and cascade removal the monitor path uses.
//! Script-managed children have no local process and rely on the monitor's
//! socket-EOF death signal instead.

use std::sync::Arc;

use super::subagent_cleanup;
use super::subagent_registry::{ExitSignal, ExitSignalTx, SubagentRegistry};
use crate::infrastructure::processes::owned_child_supervisor::{
    ChildExit, ChildHandleId, OwnedChildSupervisor,
};

pub(super) struct ReaperContext {
    pub exit_tx: ExitSignalTx,
    pub broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    pub swarm_context: Option<super::swarm_bridge::SwarmContext>,
}

pub(super) fn spawn_reaper_task(
    handle: ChildHandleId,
    supervisor: Arc<OwnedChildSupervisor>,
    registry: SubagentRegistry,
    registry_key: String,
    context: ReaperContext,
) {
    let ReaperContext {
        exit_tx,
        broadcast_tx,
        swarm_context,
    } = context;
    tokio::spawn(async move {
        let exit = supervisor.wait_exit(handle).await;
        // send_replace: store the real exit status even when no awaiter holds
        // a receiver yet, so late awaits report it instead of a fallback.
        exit_tx.send_replace(Some(exit_signal_from_exit(exit)));
        subagent_cleanup::cleanup_registered_once(&registry, &registry_key).await;
        let super::subagent_cascade::CascadeOutcome { removed, event } =
            super::subagent_cascade::cascade_remove_and_state_changed(&registry, &registry_key);
        if let Some(event) = event {
            if let Some(tx) = &broadcast_tx {
                let _ = tx.send(event);
            }
        }
        let mut removed = removed;
        subagent_cleanup::cleanup_removed_entries_once(
            &mut removed,
            subagent_cleanup::FinalizeMode::Exit,
        )
        .await;
        for (id, entry) in &removed {
            if id == &registry_key {
                if let Some(ref handle) = entry.monitor_handle {
                    handle.abort();
                }
                continue;
            }
            if let Some(ref tx) = entry.exit_signal_tx {
                let _ = tx.send(Some(ExitSignal {
                    exit_code: None,
                    signal: Some(15),
                    kind: Default::default(),
                }));
            }
            super::subagent_cascade::terminate_removed_entry(entry);
        }
        if let Some(context) = swarm_context {
            let _ = tokio::task::spawn_blocking(move || {
                if let Err(error) = super::swarm_lifecycle::reconcile(&context) {
                    tracing::error!(%error, "swarm reaper reconciliation failed; capacity retained");
                }
            }).await;
        }
        // The exit was published and every cleanup ran: the handle's slot
        // can go; retained registry clones then see "no retained handle".
        supervisor.retire(handle);
    });
}

/// The supervisor's exit report in the registry's exit-signal vocabulary.
/// An unknown handle or an unobservable wait yields the empty signal, as a
/// failed `wait` always did.
pub(super) fn exit_signal_from_exit(exit: Option<ChildExit>) -> ExitSignal {
    match exit {
        Some(ChildExit::Code(code)) => ExitSignal {
            exit_code: Some(code),
            signal: None,
            kind: Default::default(),
        },
        Some(ChildExit::Signal(signal)) => ExitSignal {
            exit_code: None,
            signal: Some(signal),
            kind: Default::default(),
        },
        Some(ChildExit::Unobservable(_)) | None => ExitSignal {
            exit_code: None,
            signal: None,
            kind: Default::default(),
        },
    }
}

#[cfg(test)]
#[path = "spawn_reaper_tests.rs"]
mod tests;
