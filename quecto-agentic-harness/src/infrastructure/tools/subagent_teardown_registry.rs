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

use super::subagent_registry::{
    ClaimOwner, ExitSignal, ExitSignalKind, NotificationTx, SequencedSubagentNotification,
    StoppingClaim, SubagentEntry, SubagentNotification, SubagentRegistry, SubagentStatus,
    TeardownIntent, TeardownPhase,
};
use crate::domain::environment_retention::MemberFinalizeMode as FinalizeMode;
use crate::infrastructure::processes::direct_child_routing::PROTOCOL_ACK_TIMEOUT;
use crate::infrastructure::processes::owned_child_supervisor::TerminationBudget;

/// The full owned-handle ladder a directly owned child's conclusion may
/// take: the protocol ACK bound, the acknowledged child's exit budget, then
/// TERM and KILL grace (`TerminationBudget::DEFAULT`).
pub const OWNED_HANDLE_LADDER: Duration = PROTOCOL_ACK_TIMEOUT
    .saturating_add(TerminationBudget::DEFAULT.exit_after_ack)
    .saturating_add(TerminationBudget::DEFAULT.term_grace)
    .saturating_add(TerminationBudget::DEFAULT.kill_grace);

/// Slack a compensation observer allows past the ladder, for the reaper's
/// own observation and compensation to run.
const COMPENSATION_WAIT_SLACK: Duration = Duration::from_secs(6);

/// How long a caller waits for a row's compensation before reporting that
/// the exit was not observed. Derived from the budgets, above the *whole*
/// owned-handle ladder (not only the exit budget), so a directly owned
/// child's fallback — including its KILL grace — always concludes before
/// a joiner gives up, and a nested target's exit, reported through its
/// ancestor's snapshot, has room to arrive.
pub const DEFAULT_COMPENSATION_WAIT: Duration =
    OWNED_HANDLE_LADDER.saturating_add(COMPENSATION_WAIT_SLACK);

pub struct RegistryDelegatedAgents {
    registry: SubagentRegistry,
    broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    notify_tx: Option<NotificationTx>,
    compensation_wait: Duration,
    /// Composition's builder of the final-member environment cleanup
    /// (#1939) a compensation runs for each membership it removes.
    finalizer: super::subagent_cleanup::MemberFinalizer,
    /// What a natural exit's note says about why the child ended (#2192):
    /// the crash record it left. `None`, the note names only the
    /// observation.
    ended: Option<std::sync::Arc<crate::application::subagents::use_cases::InspectEndedChild>>,
}

/// What an exit note adds when the child's transcript can be read.
pub const TRANSCRIPT_STAYS_READABLE: &str =
    "Its transcript up to the end stays readable with agent_cmd get_messages";

impl RegistryDelegatedAgents {
    pub fn new(
        registry: SubagentRegistry,
        broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
        notify_tx: Option<NotificationTx>,
        finalizer: super::subagent_cleanup::MemberFinalizer,
    ) -> Self {
        Self {
            registry,
            broadcast_tx,
            notify_tx,
            compensation_wait: DEFAULT_COMPENSATION_WAIT,
            finalizer,
            ended: None,
        }
    }

    /// Read why a child ended on its own for the note its exit posts.
    pub fn with_ended_child(
        mut self,
        ended: Option<std::sync::Arc<crate::application::subagents::use_cases::InspectEndedChild>>,
    ) -> Self {
        self.ended = ended;
        self
    }

    /// How the target ended, for its natural-exit note, in the words every
    /// other view of its end uses: its exit status (what the reaper
    /// published, for a child this harness held) and the crash record it
    /// left. `None` when nothing beyond the end is known.
    async fn end_detail(
        &self,
        target: Option<&SubagentEntry>,
        observation: &str,
    ) -> Option<String> {
        let (ended, target) = (self.ended.as_ref()?, target?);
        let exit = target
            .exit_signal_tx
            .as_ref()
            .and_then(|tx| tx.borrow().clone());
        let crash = ended
            .crash(
                &target.agent_uuid,
                target.origin,
                super::agent_cmd_ended::vouched_pid(target),
            )
            .await;
        let end = super::agent_cmd_ended::child_end_of(target, exit.as_ref(), crash);
        // Nothing observed and nothing left: the note's own wording says so.
        // How the end was observed stays in the note (#2192 review), as it
        // does when nothing else is known.
        let reason = match (end.kind(), &end.crash) {
            (crate::domain::child_end::EndKind::Unknown, None) => return None,
            _ => format!(
                "{} ({})",
                end.reason(),
                crate::domain::child_end::shown(observation, 64)
            ),
        };
        // The transcript is offered only when it can be read (#2192
        // review): a child this harness launched, whose store is this one's.
        match ended
            .has_transcript(&target.agent_uuid, target.origin)
            .await
        {
            true => Some(format!("{reason}. {TRANSCRIPT_STAYS_READABLE}")),
            false => Some(reason),
        }
    }

    /// How long a natural exit's note waits for the reaper to publish an
    /// owned child's exit status (#2260). Red-phase stub.
    pub fn with_exit_status_wait(self, _wait: Duration) -> Self {
        self
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
        let key = match super::subagent_registry::resolve_registry_key(&entries, reference) {
            Ok(key) => key,
            Err(DisplayNameResolveError::AmbiguousLiveMatch { .. }) => {
                return Err(ResolutionError::Ambiguous);
            }
            // No live row answers to the label: a retained dead row that
            // does is exited, not unknown.
            Err(DisplayNameResolveError::NoLiveMatch { .. }) => {
                let exited = entries
                    .iter()
                    .any(|(key, entry)| entry.effective_display_name(key) == reference);
                return Err(if exited {
                    ResolutionError::Exited
                } else {
                    ResolutionError::Unknown
                });
            }
        };
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
                    .send_replace(TeardownPhase::Stopping(StoppingClaim::first(intent_of(
                        cause,
                    ))));
                Ok(())
            }
            TeardownPhase::Live => Err(StoppingClaimError::Exited),
            // The earlier owner returned without observing the end: this
            // trigger re-takes the claim under its own intent and
            // re-attempts; the row was never released in between, so the
            // exit is still compensated as a termination, never a
            // post-mortem.
            TeardownPhase::Stopping(StoppingClaim {
                attempt,
                owner: ClaimOwner::Returned,
                ..
            }) => {
                entry
                    .teardown
                    .send_replace(TeardownPhase::Stopping(StoppingClaim {
                        intent: intent_of(cause),
                        attempt: attempt.saturating_add(1),
                        owner: ClaimOwner::Executing,
                    }));
                Ok(())
            }
            TeardownPhase::Stopping(StoppingClaim {
                owner: ClaimOwner::Executing,
                ..
            }) => Err(StoppingClaimError::AlreadyStopping),
            TeardownPhase::Compensating(_) | TeardownPhase::Compensated => {
                Err(StoppingClaimError::Exited)
            }
        }
    }

    fn retain_stopping(&self, target: &DelegatedAgentIdentity) {
        let entries = self.lock();
        if let Some(entry) = Self::key_for(&entries, &target.uuid).and_then(|key| entries.get(&key))
        {
            if let TeardownPhase::Stopping(claim) = entry.teardown_phase() {
                entry
                    .teardown
                    .send_replace(TeardownPhase::Stopping(StoppingClaim {
                        owner: ClaimOwner::Returned,
                        ..claim
                    }));
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
            TeardownPhase::Stopping(claim) => {
                entry
                    .teardown
                    .send_replace(TeardownPhase::Compensating(claim.intent));
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
        TerminationCause::RunEnd => TeardownIntent::RunEnd,
        TerminationCause::FleetTeardown => TeardownIntent::FleetTeardown,
        TerminationCause::OwnerTeardown => TeardownIntent::OwnerTeardown,
        TerminationCause::EnvironmentKill => TeardownIntent::EnvironmentKill,
        TerminationCause::LaunchRollback { owns_environment } => {
            TeardownIntent::LaunchRollback { owns_environment }
        }
    }
}

/// The cause the compensation honours: an exit observed by the reaper or
/// the monitor for a row a kill, fleet teardown or rollback had already
/// claimed stopping is that termination's end, not a post-mortem.
fn effective_cause(entry: &SubagentEntry, cause: TerminationCause) -> TerminationCause {
    match (entry.teardown_phase(), cause) {
        (
            TeardownPhase::Compensating(TeardownIntent::SelectedTermination),
            TerminationCause::Exit(_),
        ) => TerminationCause::SelectedTermination,
        (TeardownPhase::Compensating(TeardownIntent::RunEnd), TerminationCause::Exit(_)) => {
            TerminationCause::RunEnd
        }
        (TeardownPhase::Compensating(TeardownIntent::FleetTeardown), TerminationCause::Exit(_)) => {
            TerminationCause::FleetTeardown
        }
        (TeardownPhase::Compensating(TeardownIntent::OwnerTeardown), TerminationCause::Exit(_)) => {
            TerminationCause::OwnerTeardown
        }
        (
            TeardownPhase::Compensating(TeardownIntent::EnvironmentKill),
            TerminationCause::Exit(_),
        ) => TerminationCause::EnvironmentKill,
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
        // #2206: the owner ending this one child — its selected kill, or a
        // one-shot parent ending its run — is its word: a plain container
        // child's box goes for good; a swarm it has not closed is kept.
        TerminationCause::SelectedTermination | TerminationCause::RunEnd => FinalizeMode::OwnerEnd,
        // A harness shutdown can be a crash (a signal, its last client gone,
        // a lost parent): it keeps a swarm that has not ended (#2070). The
        // member side of an environment kill runs under the kill's own
        // claim.
        TerminationCause::FleetTeardown | TerminationCause::EnvironmentKill => {
            FinalizeMode::ParentKill
        }
        // #2070: delete-all, or a session transition whose target is proven,
        // is the owner ending the swarms it holds; their containers go.
        TerminationCause::OwnerTeardown => FinalizeMode::OwnerTeardown,
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
        TerminationCause::SelectedTermination
        | TerminationCause::RunEnd
        | TerminationCause::FleetTeardown
        | TerminationCause::OwnerTeardown
        | TerminationCause::EnvironmentKill
        | TerminationCause::LaunchRollback { .. } => ExitSignalKind::Terminated,
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
            super::subagent_cleanup::cleanup_registered_once(
                &self.registry,
                key,
                mode,
                self.finalizer,
            )
            .await;
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
            // Rows that were no longer live before this compensation ran
            // (a descendant that already ended, or the target itself when
            // its exit was recorded first) leave the registry with the
            // subtree but are neither cleaned up nor signalled again.
            let (mut removed, already_ended): (Vec<_>, Vec<_>) = removed
                .into_iter()
                .partition(|(id, _)| live_before.contains(id));
            super::subagent_cleanup::cleanup_removed_entries_once(
                &mut removed,
                mode,
                self.finalizer,
            )
            .await;
            let kind = exit_kind(cause);
            // Why a child ended on its own, read before the target's exit
            // signal is replaced below: what the reaper published for a
            // child this harness held, and the crash record it left (#2192).
            let detail = match cause {
                TerminationCause::Exit(_) => {
                    let target = removed
                        .iter()
                        .chain(&already_ended)
                        .find(|(id, _)| id == key)
                        .map(|(_, entry)| entry);
                    self.end_detail(target, kind.to_wire_str()).await
                }
                _ => None,
            };
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
                            detail,
                        },
                        agent_uuid,
                    ));
                }
            }
            debug_assert!(
                removed.first().is_none_or(|(id, _)| id == key),
                "the cascade lists the target first"
            );
            // Every terminal effect has run: only now is the row (and each
            // row that fell with it) compensated for whoever waits on it —
            // the already-ended rows too, since nothing further will ever
            // run for a row that has left the registry.
            for (_, entry) in removed.iter().chain(&already_ended) {
                super::subagent_cascade::mark_entry_compensated(entry);
            }
            Compensated {
                removed: removed
                    .iter()
                    .map(|(_, entry)| entry.agent_uuid.clone())
                    .collect(),
            }
        })
    }

    /// A terminal row is one whose ladder reached `Compensated`: its
    /// terminal effects have run, nothing is retained for it, and only its
    /// record remains. Liveness alone is not enough — `compensate` marks a
    /// row dead while its phase is still `Compensating`, and pruning it
    /// then would leave the cascade nothing to remove, the monitor running
    /// and the exit signal unfired (review of #1938). Live and in-flight
    /// rows are untouched, so a teardown still settling a child can never
    /// lose it here. The survivor set does not change (dead rows are not
    /// listed), so nothing is broadcast.
    fn prune_terminal_rows(&self) -> PortFuture<'_, Vec<crate::domain::ids::AgentUuid>> {
        Box::pin(async move {
            let mut entries = self.lock();
            let mut terminal: Vec<String> = entries
                .iter()
                .filter(|(_, entry)| entry.teardown_phase() == TeardownPhase::Compensated)
                .map(|(key, _)| key.clone())
                .collect();
            terminal.sort();
            terminal
                .iter()
                .filter_map(|key| entries.remove(key))
                .map(|entry| entry.agent_uuid)
                .collect()
        })
    }
}

#[cfg(test)]
#[path = "subagent_teardown_registry_2192_tests.rs"]
mod end_detail_tests;
#[cfg(test)]
#[path = "subagent_teardown_registry_tests.rs"]
mod tests;
