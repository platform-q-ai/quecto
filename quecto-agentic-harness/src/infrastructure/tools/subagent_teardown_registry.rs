//! The subagent registry as the [`DelegatedAgentRegistry`] and
//! [`TeardownCompensation`] ports (#1936).
//!
//! The registry row is the one place every termination path meets: an
//! operator kill, the reaper's exit report, the monitor's EOF, a launch
//! rollback and a reported-snapshot prune all claim their way through the
//! row's [`TeardownPhase`], so the terminal effects — environment cleanup
//! and membership removal, monitor and bridge teardown, removal of the row
//! and its reported subtree from live membership, one survivor broadcast,
//! one exit signal per removed row and one passive note — run exactly once
//! and only after the exit they stand for was observed. Nothing here ever
//! signals a process.
use std::time::Duration;

use crate::application::subagents::ports::{
    Compensated, CompensationObservation, DelegatedAgentRegistry, ExitObservation, PortFuture,
    ResolutionError, StoppingClaimError, TeardownCompensation, TerminalClaim, TerminationCause,
};
use crate::domain::session::SubagentLiveness;
use crate::domain::subagent::DisplayNameResolveError;
use crate::domain::subagent_teardown::DelegatedAgentIdentity;

use super::subagent_cleanup::FinalizeMode;
use super::subagent_registry::{
    ExitSignal, ExitSignalKind, NotificationTx, SequencedSubagentNotification, SubagentEntry,
    SubagentNotification, SubagentRegistry, SubagentStatus, TeardownIntent, TeardownPhase,
};

/// How long a caller waits for a row's compensation before reporting that
/// the exit was not observed. Sized above the owned-handle exit budget
/// (`TerminationBudget::DEFAULT`, 10 s after an ACK) so a directly owned
/// child's fallback always concludes first, and a nested target's exit,
/// reported through its ancestor's snapshot, has room to arrive.
pub const DEFAULT_COMPENSATION_WAIT: Duration = Duration::from_secs(15);

pub struct RegistryDelegatedAgents {
    registry: SubagentRegistry,
    broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    notify_tx: Option<NotificationTx>,
    compensation_wait: Duration,
}

impl RegistryDelegatedAgents {
    pub fn new(
        registry: SubagentRegistry,
        broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
        notify_tx: Option<NotificationTx>,
    ) -> Self {
        Self {
            registry,
            broadcast_tx,
            notify_tx,
            compensation_wait: DEFAULT_COMPENSATION_WAIT,
        }
    }

    pub fn with_compensation_wait(mut self, wait: Duration) -> Self {
        self.compensation_wait = wait;
        self
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, std::collections::HashMap<String, SubagentEntry>> {
        self.registry.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The registry key of the row carrying `uuid`. Production rows are
    /// keyed by their uuid; hand-built rows may be keyed by a label, so the
    /// scan is the fallback, never the first answer.
    fn key_for(
        entries: &std::collections::HashMap<String, SubagentEntry>,
        uuid: &crate::domain::ids::AgentUuid,
    ) -> Option<String> {
        if entries.contains_key(uuid.as_str()) {
            return Some(uuid.as_str().to_owned());
        }
        entries
            .iter()
            .find(|(_, entry)| &entry.agent_uuid == uuid)
            .map(|(key, _)| key.clone())
    }

    /// A row is only addressable while it is live in every sense the
    /// registry tracks: not exited, not dead, not already compensated.
    fn is_live(entry: &SubagentEntry) -> bool {
        entry.persisted_liveness == SubagentLiveness::Live
            && entry.status != SubagentStatus::Exited
            && !matches!(
                entry.teardown_phase(),
                TeardownPhase::Compensating(_) | TeardownPhase::Compensated
            )
    }
}

impl DelegatedAgentRegistry for RegistryDelegatedAgents {
    fn resolve(&self, reference: &str) -> Result<DelegatedAgentIdentity, ResolutionError> {
        let entries = self.lock();
        let key = super::subagent_registry::resolve_registry_key(&entries, reference).map_err(
            |error| match error {
                DisplayNameResolveError::AmbiguousLiveMatch { .. } => ResolutionError::Ambiguous,
                DisplayNameResolveError::NoLiveMatch { .. } => ResolutionError::Unknown,
            },
        )?;
        let entry = entries.get(&key).ok_or(ResolutionError::Unknown)?;
        if !Self::is_live(entry) {
            return Err(ResolutionError::Exited);
        }
        entry
            .delegated_identity()
            .ok_or(ResolutionError::NotDelegated)
    }

    fn claim_stopping(
        &self,
        target: &DelegatedAgentIdentity,
        cause: TerminationCause,
    ) -> Result<(), StoppingClaimError> {
        let entries = self.lock();
        let entry = Self::key_for(&entries, &target.uuid)
            .and_then(|key| entries.get(&key))
            .filter(|entry| entry.delegated_identity().as_ref() == Some(target))
            .ok_or(StoppingClaimError::Unknown)?;
        match entry.teardown_phase() {
            TeardownPhase::Live if Self::is_live(entry) => {
                entry
                    .teardown
                    .send_replace(TeardownPhase::Stopping(intent_of(cause)));
                Ok(())
            }
            TeardownPhase::Live => Err(StoppingClaimError::Exited),
            TeardownPhase::Stopping(_) => Err(StoppingClaimError::AlreadyStopping),
            TeardownPhase::Compensating(_) | TeardownPhase::Compensated => {
                Err(StoppingClaimError::Exited)
            }
        }
    }

    fn release_stopping(&self, target: &DelegatedAgentIdentity) {
        let entries = self.lock();
        if let Some(entry) = Self::key_for(&entries, &target.uuid).and_then(|key| entries.get(&key))
        {
            if matches!(entry.teardown_phase(), TeardownPhase::Stopping(_)) {
                entry.teardown.send_replace(TeardownPhase::Live);
            }
        }
    }

    fn claim_terminal(&self, target: &DelegatedAgentIdentity) -> TerminalClaim {
        let entries = self.lock();
        let Some(entry) = Self::key_for(&entries, &target.uuid).and_then(|key| entries.get(&key))
        else {
            return TerminalClaim::AlreadyClaimed;
        };
        match entry.teardown_phase() {
            TeardownPhase::Live => {
                entry
                    .teardown
                    .send_replace(TeardownPhase::Compensating(TeardownIntent::Exit));
                TerminalClaim::Claimed
            }
            TeardownPhase::Stopping(intent) => {
                entry
                    .teardown
                    .send_replace(TeardownPhase::Compensating(intent));
                TerminalClaim::Claimed
            }
            TeardownPhase::Compensating(_) | TeardownPhase::Compensated => {
                TerminalClaim::AlreadyClaimed
            }
        }
    }

    fn holds_process(&self, target: &DelegatedAgentIdentity) -> bool {
        let entries = self.lock();
        Self::key_for(&entries, &target.uuid)
            .and_then(|key| entries.get(&key))
            .is_some_and(SubagentEntry::holds_owned_child)
    }

    fn terminal_claimed(&self, target: &DelegatedAgentIdentity) -> bool {
        let entries = self.lock();
        Self::key_for(&entries, &target.uuid)
            .and_then(|key| entries.get(&key))
            .is_some_and(|entry| {
                matches!(
                    entry.teardown_phase(),
                    TeardownPhase::Compensating(_) | TeardownPhase::Compensated
                )
            })
    }

    fn await_compensated<'a>(
        &'a self,
        target: &'a DelegatedAgentIdentity,
    ) -> PortFuture<'a, CompensationObservation> {
        Box::pin(async move {
            let mut phases = {
                let entries = self.lock();
                let Some(entry) =
                    Self::key_for(&entries, &target.uuid).and_then(|key| entries.get(&key))
                else {
                    return CompensationObservation::Unknown;
                };
                entry.teardown.subscribe()
            };
            let wait = async {
                loop {
                    if *phases.borrow_and_update() == TeardownPhase::Compensated {
                        return CompensationObservation::Compensated;
                    }
                    if phases.changed().await.is_err() {
                        // Every sender dropped: the row was uncommitted.
                        return CompensationObservation::Unknown;
                    }
                }
            };
            tokio::time::timeout(self.compensation_wait, wait)
                .await
                .unwrap_or(CompensationObservation::TimedOut)
        })
    }
}

fn intent_of(cause: TerminationCause) -> TeardownIntent {
    match cause {
        TerminationCause::Exit(_) => TeardownIntent::Exit,
        TerminationCause::SelectedTermination => TeardownIntent::SelectedTermination,
        TerminationCause::LaunchRollback { owns_environment } => {
            TeardownIntent::LaunchRollback { owns_environment }
        }
    }
}

/// The cause the compensation honours: an exit observed by the reaper or
/// the monitor for a row a kill or rollback had already claimed stopping
/// is that termination's end, not a post-mortem.
fn effective_cause(entry: &SubagentEntry, cause: TerminationCause) -> TerminationCause {
    match (entry.teardown_phase(), cause) {
        (
            TeardownPhase::Compensating(TeardownIntent::SelectedTermination),
            TerminationCause::Exit(_),
        ) => TerminationCause::SelectedTermination,
        (
            TeardownPhase::Compensating(TeardownIntent::LaunchRollback { owns_environment }),
            TerminationCause::Exit(_),
        ) => TerminationCause::LaunchRollback { owns_environment },
        (_, cause) => cause,
    }
}

/// The cleanup contract a cause maps to: a natural exit is a post-mortem
/// (inspect runs), a parent-initiated end is not, and a rollback runs the
/// retained `cleanup` instead of `kill`.
fn finalize_mode(cause: TerminationCause) -> FinalizeMode {
    match cause {
        TerminationCause::Exit(_) => FinalizeMode::Exit,
        TerminationCause::SelectedTermination => FinalizeMode::ParentKill,
        TerminationCause::LaunchRollback {
            owns_environment: true,
        } => FinalizeMode::LaunchRollbackOwned,
        TerminationCause::LaunchRollback {
            owns_environment: false,
        } => FinalizeMode::LaunchRollback,
    }
}

fn exit_kind(cause: TerminationCause) -> ExitSignalKind {
    match cause {
        TerminationCause::Exit(ExitObservation::ProcessExited) => ExitSignalKind::ProcessExit,
        TerminationCause::Exit(ExitObservation::ConnectionClosed) => {
            ExitSignalKind::ConnectionClosed
        }
        TerminationCause::Exit(ExitObservation::NeverReachable) => ExitSignalKind::NeverReachable,
        TerminationCause::SelectedTermination | TerminationCause::LaunchRollback { .. } => {
            ExitSignalKind::Terminated
        }
    }
}

impl TeardownCompensation for RegistryDelegatedAgents {
    fn compensate<'a>(
        &'a self,
        target: &'a DelegatedAgentIdentity,
        cause: TerminationCause,
    ) -> PortFuture<'a, Compensated> {
        Box::pin(async move {
            let (key, cause, live_before) = {
                let entries = self.lock();
                let Some(key) = Self::key_for(&entries, &target.uuid) else {
                    return Compensated {
                        removed: Vec::new(),
                    };
                };
                let cause = effective_cause(&entries[&key], cause);
                // Only rows this compensation moves out of live membership
                // are reported (and signalled): a descendant that already
                // ended keeps its own record and is not ended twice.
                let live_before: std::collections::HashSet<String> = entries
                    .iter()
                    .filter(|(_, entry)| {
                        entry.status != SubagentStatus::Exited
                            && entry.persisted_liveness == SubagentLiveness::Live
                    })
                    .map(|(key, _)| key.clone())
                    .collect();
                (key, cause, live_before)
            };
            let key = key.as_str();
            let mode = finalize_mode(cause);
            // Membership removal and per-entry cleanup claims run BEFORE
            // the row is marked exited and before any signal fires: a woken
            // observer must see the authoritative environment aggregate
            // already updated.
            super::subagent_cleanup::cleanup_registered_once(&self.registry, key, mode).await;
            let sequence = super::subagent_monitor::update_entry_next_sequence(
                &self.registry,
                key,
                super::subagent_monitor::mark_exited,
            );
            {
                let mut entries = self.lock();
                if let Some(entry) = entries.get_mut(key) {
                    super::spawn_proxy_bridge::teardown_entry_bridge(
                        entry.proxy_bridge_handle.take().as_ref(),
                        entry.proxy_bridge_socket.take().as_deref(),
                    );
                }
            }
            let label = super::subagent_monitor::notification_display_label(&self.registry, key);
            let agent_uuid = super::subagent_monitor::notification_agent_uuid(&self.registry, key);
            let super::subagent_cascade::CascadeOutcome { removed, event } =
                super::subagent_cascade::cascade_remove_and_state_changed(&self.registry, key);
            if let (Some(event), Some(tx)) = (event, self.broadcast_tx.as_ref()) {
                // One survivor-only roster per compensation.
                let _ = tx.send(event);
            }
            let mut removed: Vec<_> = removed
                .into_iter()
                .filter(|(id, _)| live_before.contains(id))
                .collect();
            super::subagent_cleanup::cleanup_removed_entries_once(&mut removed, mode).await;
            let kind = exit_kind(cause);
            for (id, entry) in &removed {
                if let Some(ref handle) = entry.monitor_handle {
                    handle.abort();
                }
                // Descendants fell with the subtree; they carry no exit
                // status of their own. The target's own signal is the
                // reaper's when this harness held its process.
                let is_target = id == key;
                if let Some(ref tx) = entry.exit_signal_tx {
                    if !is_target {
                        tx.send_replace(Some(ExitSignal {
                            exit_code: None,
                            signal: None,
                            kind: ExitSignalKind::Terminated,
                        }));
                    } else if entry.owned_child.is_none() {
                        tx.send_replace(Some(ExitSignal {
                            exit_code: None,
                            signal: None,
                            kind,
                        }));
                    }
                }
            }
            if matches!(cause, TerminationCause::Exit(_)) {
                if let Some(tx) = self.notify_tx.as_ref() {
                    let _ = tx.try_send(SequencedSubagentNotification::new_for_agent(
                        sequence,
                        SubagentNotification::Exited {
                            agent_id: label,
                            reason: Some(kind.to_wire_str().to_string()),
                        },
                        agent_uuid,
                    ));
                }
            }
            debug_assert!(
                removed.first().is_none_or(|(id, _)| id == key),
                "the cascade lists the target first"
            );
            Compensated {
                removed: removed
                    .iter()
                    .map(|(_, entry)| entry.agent_uuid.clone())
                    .collect(),
            }
        })
    }
}

#[cfg(test)]
#[path = "subagent_teardown_registry_tests.rs"]
mod tests;
