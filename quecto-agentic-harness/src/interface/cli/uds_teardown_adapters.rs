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

    pub fn signalled(&self) -> Option<ExitReadiness> {
        self.readiness
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

impl CompositionExitReadiness for LoopExitReadiness {
    fn signal_exit_ready(&self, readiness: ExitReadiness) -> PortFuture<'_, ()> {
        Box::pin(async move {
            *self.readiness.lock().unwrap_or_else(|e| e.into_inner()) = Some(readiness);
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

/// Lifecycle state of this harness plus its direct children as recorded in
/// the subagent registry: a row is a direct child exactly when this harness
/// launched it (it carries a launch generation). Merged descendants and
/// restored rows carry none and are not this harness's to address.
pub struct RegistryLifecycleRepository {
    state: Mutex<HarnessLifecycleState>,
    registry: Option<SubagentRegistry>,
    owner: AgentUuid,
}

impl RegistryLifecycleRepository {
    pub fn new(registry: Option<SubagentRegistry>, owner: AgentUuid) -> Self {
        Self {
            state: Mutex::new(HarnessLifecycleState::Accepting),
            registry,
            owner,
        }
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
                let Some(generation) = entry.launch_generation else {
                    continue;
                };
                records.push(LineageRecord {
                    identity: DelegatedAgentIdentity::new(entry.agent_uuid.clone(), generation),
                    parent: self.owner.clone(),
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
