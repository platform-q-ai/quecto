//! Loop-bound adapters for the subagent teardown ports (#1935).
//!
//! These implement the capability-local ports of `application/subagents`
//! against the multi-client dispatch loop's own primitives: the cancel slot
//! and turn control (turn cancellation), the loop's exit `Notify`
//! (composition exit readiness), the tokio runtime (run spawner), a
//! monotonic clock, and the subagent registry (lifecycle + lineage). They
//! translate; none of them decides what a shutdown does.
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

use crate::application::subagents::ports::{
    CompositionExitReadiness, ExitReadiness, PortFuture, ShutdownClock, ShutdownInstant,
    ShutdownRun, ShutdownRunSpawner, ShutdownSessionPersistence, SubagentLifecycleRepository,
    TurnCancellation,
};
use crate::domain::ids::AgentUuid;
use crate::domain::subagent_teardown::{
    DelegatedAgentIdentity, HarnessLifecycleState, LineageRecord, LineageSnapshot, ShutdownReason,
};
use crate::infrastructure::tools::harness_lifecycle::SharedHarnessLifecycle;
use crate::infrastructure::tools::subagent_registry::SubagentRegistry;
use crate::interface::uds::subagent_teardown::presenter::{AckWriteError, AckWriter};
use crate::interface::uds::subagent_teardown::wire::TeardownResponse;

use super::uds_cancel::{CancelHandle, TurnControlHandle, fire_cancel};
use super::uds_multi::BusyFlag;
use super::uds_wire::ConnectionWireMode;

/// Cancels the in-flight turn the way the reader task does for `abort`:
/// record the operator intent, then fire the cancel slot.
pub(crate) struct LoopTurnCancellation {
    pub(crate) cancel_handle: CancelHandle,
    pub(crate) turn_control: TurnControlHandle,
    pub(crate) busy: BusyFlag,
}

impl TurnCancellation for LoopTurnCancellation {
    fn cancel_in_flight_turn(&self) -> PortFuture<'_, bool> {
        Box::pin(async move {
            let was_busy = self.busy.load(std::sync::atomic::Ordering::SeqCst);
            // Close turn admission first, then cancel the in-flight turn:
            // whatever the loop does at the idle boundary that follows (a
            // drained follow-up, a subagent-note nudge) can no longer start
            // a turn, so it reaches the exit signal promptly (#1936).
            self.turn_control.mark_shutting_down();
            self.turn_control.mark_abort();
            fire_cancel(&self.cancel_handle);
            was_busy
        })
    }
}

/// Session persistence is owned by the dispatch loop's exit path (which the
/// exit-readiness signal below triggers, with an empty roster because the
/// subtree was torn down). This adapter records the reason it will persist
/// for and reports success; a loop that cannot save reports that itself.
#[derive(Default)]
pub struct DeferredLoopPersistence {
    reason: Mutex<Option<ShutdownReason>>,
}

impl DeferredLoopPersistence {
    pub fn recorded_reason(&self) -> Option<ShutdownReason> {
        *self.reason.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl ShutdownSessionPersistence for DeferredLoopPersistence {
    fn persist_for_shutdown(&self, reason: ShutdownReason) -> PortFuture<'_, Result<(), String>> {
        Box::pin(async move {
            *self.reason.lock().unwrap_or_else(|e| e.into_inner()) = Some(reason);
            Ok(())
        })
    }
}

/// Tells the dispatch loop to finish: the same `Notify` the termination
/// signal watcher uses, so both converge on one exit path.
pub struct LoopExitReadiness {
    notify: Arc<Notify>,
    readiness: Mutex<Option<ExitReadiness>>,
}

impl LoopExitReadiness {
    pub fn new(notify: Arc<Notify>) -> Self {
        Self {
            notify,
            readiness: Mutex::new(None),
        }
    }

    pub fn readiness_signalled(&self) -> Option<ExitReadiness> {
        self.readiness
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

/// Test hook (#1936): when set to `1` in a launched child's environment,
/// the child records its exit readiness but never leaves — a harness whose
/// teardown acknowledged and then stalled — so the parent's owned-handle
/// fallback can be proven against a real process. Compiled only with
/// `test-support` (the real-process tests build the binary with it); a
/// production binary has no such gate.
#[cfg(any(test, feature = "test-support"))]
pub const HOLD_EXIT_AFTER_ACK_ENV: &str = "QUECTO_TEST_HOLD_EXIT_AFTER_ACK";

/// Whether the test hook holds this exit. Always `false` in a production
/// build: the environment is not consulted.
fn exit_held_by_test_hook() -> bool {
    #[cfg(any(test, feature = "test-support"))]
    {
        if std::env::var(HOLD_EXIT_AFTER_ACK_ENV).as_deref() == Ok("1") {
            tracing::warn!("exit readiness held by {HOLD_EXIT_AFTER_ACK_ENV}");
            return true;
        }
    }
    false
}

impl CompositionExitReadiness for LoopExitReadiness {
    fn signal_exit_ready(&self, readiness: ExitReadiness) -> PortFuture<'_, ()> {
        Box::pin(async move {
            *self.readiness.lock().unwrap_or_else(|e| e.into_inner()) = Some(readiness);
            if exit_held_by_test_hook() {
                return;
            }
            self.notify.notify_one();
        })
    }
}

/// Detaches the teardown run onto the current tokio runtime.
pub(crate) struct TokioShutdownRunSpawner;

impl ShutdownRunSpawner for TokioShutdownRunSpawner {
    fn spawn_shutdown_run(&self, run: ShutdownRun) {
        tokio::spawn(run);
    }
}

pub(crate) struct MonotonicShutdownClock {
    epoch: std::time::Instant,
}

impl Default for MonotonicShutdownClock {
    fn default() -> Self {
        Self {
            epoch: std::time::Instant::now(),
        }
    }
}

impl ShutdownClock for MonotonicShutdownClock {
    fn now(&self) -> ShutdownInstant {
        ShutdownInstant(u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX))
    }
}

/// Lifecycle state of this harness plus its delegation lineage as recorded
/// in the subagent registry: a row is a direct child exactly when this
/// harness launched it (it carries a launch generation); a merged descendant
/// reported with its launch generation is a deeper record parented by the
/// row that reported it (#1936), reachable only by forwarding. Restored rows
/// and fixtures carry no generation and are not this harness's to address.
///
/// The lifecycle state lives in the harness's shared cell (#1938): the
/// spawn tool reads it under the registry lock when it registers a child,
/// so a freeze recorded here is what refuses a spawn racing the shutdown.
pub struct RegistryLifecycleRepository {
    state: SharedHarnessLifecycle,
    registry: Option<SubagentRegistry>,
    owner: AgentUuid,
}

impl RegistryLifecycleRepository {
    /// Over the given shared cell, which the spawn tool of the same harness
    /// must read for admission.
    pub fn new(
        registry: Option<SubagentRegistry>,
        owner: AgentUuid,
        state: SharedHarnessLifecycle,
    ) -> Self {
        Self {
            state,
            registry,
            owner,
        }
    }

    /// The shared cell, for a caller that must read it under another lock.
    pub fn shared_state(&self) -> SharedHarnessLifecycle {
        self.state.clone()
    }
}

impl SubagentLifecycleRepository for RegistryLifecycleRepository {
    fn lifecycle(&self) -> HarnessLifecycleState {
        *self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set_lifecycle(&self, state: HarnessLifecycleState) {
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = state;
    }

    fn lineage(&self) -> LineageSnapshot {
        let mut records = Vec::new();
        if let Some(registry) = &self.registry {
            let entries = registry.lock().unwrap_or_else(|e| e.into_inner());
            let mut keys: Vec<_> = entries.keys().cloned().collect();
            keys.sort();
            for key in keys {
                let entry = &entries[&key];
                // A terminal row is no edge: a target that already ended
                // is refused as unknown rather than routed toward.
                if entry.persisted_liveness == crate::domain::session::SubagentLiveness::Dead
                    || entry.status
                        == crate::infrastructure::tools::subagent_registry::SubagentStatus::Exited
                {
                    continue;
                }
                if let Some(generation) = entry.launch_generation {
                    records.push(LineageRecord {
                        identity: DelegatedAgentIdentity::new(entry.agent_uuid.clone(), generation),
                        parent: self.owner.clone(),
                    });
                    continue;
                }
                // A descendant reported with its launch generation (#1936)
                // is addressable one edge at a time through the row that
                // reported it as parent; a row claiming this harness as its
                // parent without having been launched here is no edge.
                let (Some(generation), Some(parent)) =
                    (entry.reported_generation, entry.parent_id.as_deref())
                else {
                    continue;
                };
                if parent == self.owner.as_str() {
                    continue;
                }
                records.push(LineageRecord {
                    identity: DelegatedAgentIdentity::new(entry.agent_uuid.clone(), generation),
                    parent: AgentUuid::new(parent),
                });
            }
        }
        LineageSnapshot {
            owner: self.owner.clone(),
            records,
        }
    }
}

/// The client connection's write half, shared between its broadcast writer
/// task and the teardown edge so an ACK is written and flushed in order.
pub(crate) type SharedWriter =
    Arc<tokio::sync::Mutex<tokio::io::WriteHalf<tokio::net::UnixStream>>>;

/// Writes one teardown response in the connection's negotiated framing and
/// flushes it before resolving.
pub(crate) struct ConnectionAckWriter {
    pub(crate) writer: SharedWriter,
    pub(crate) mode: ConnectionWireMode,
}

impl AckWriter for ConnectionAckWriter {
    fn write_and_flush<'a>(
        &'a self,
        response: &'a TeardownResponse,
    ) -> Pin<Box<dyn Future<Output = Result<(), AckWriteError>> + Send + 'a>> {
        Box::pin(async move {
            use tokio::io::AsyncWriteExt;
            let line = response.to_line();
            let mut writer = self.writer.lock().await;
            super::uds_wire::write_event_line(&mut *writer, &line, &self.mode)
                .await
                .map_err(|e| AckWriteError(e.to_string()))?;
            writer
                .flush()
                .await
                .map_err(|e| AckWriteError(e.to_string()))
        })
    }
}

#[cfg(test)]
#[path = "uds_teardown_adapters_tests.rs"]
mod tests;
