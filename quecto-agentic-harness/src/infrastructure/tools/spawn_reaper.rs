//! Reaper for locally spawned subagent children.
//!
//! Local launches (only) own an OS child process; this task waits for it,
//! translates its exit status into the shared exit signal, and drives the
//! same exactly-once cleanup and cascade removal the monitor path uses.
//! Script-managed children have no local process and rely on the monitor's
//! socket-EOF death signal instead; Slice 3 unifies the two paths.

use super::subagent_cleanup;
use super::subagent_registry::{ExitSignal, ExitSignalTx, SubagentRegistry};

pub(super) fn spawn_reaper_task(
    mut child: tokio::process::Child,
    registry: SubagentRegistry,
    registry_key: String,
    exit_tx: ExitSignalTx,
    broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    ownership: super::process_ownership::ProcessOwnership,
) {
    tokio::spawn(async move {
        let status = ownership.wait(&mut child).await;
        // send_replace: store the real exit status even when no awaiter holds
        // a receiver yet, so late awaits report it instead of a fallback.
        exit_tx.send_replace(Some(exit_signal_from_status(status)));
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
    });
}

fn exit_signal_from_status(status: std::io::Result<std::process::ExitStatus>) -> ExitSignal {
    match status {
        Ok(exit_status) => {
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                if let Some(signal) = exit_status.signal() {
                    ExitSignal {
                        exit_code: None,
                        signal: Some(signal),
                        kind: Default::default(),
                    }
                } else {
                    ExitSignal {
                        exit_code: exit_status.code(),
                        signal: None,
                        kind: Default::default(),
                    }
                }
            }
            #[cfg(not(unix))]
            {
                ExitSignal {
                    exit_code: exit_status.code(),
                    signal: None,
                    kind: Default::default(),
                }
            }
        }
        Err(_) => ExitSignal {
            exit_code: None,
            signal: None,
            kind: Default::default(),
        },
    }
}

#[cfg(test)]
#[path = "spawn_reaper_tests.rs"]
mod tests;
