//! Port fakes shared by the teardown use-case tests. Test-only.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::domain::ids::AgentUuid;
use crate::domain::subagent_teardown::{
    DelegatedAgentIdentity, HarnessLifecycleState, LaunchGeneration, LineageRecord,
    LineageSnapshot, RoutingDepth, ShutdownReason,
};

use crate::application::subagents::ports::{
    ChildRoutingError, CompositionExitReadiness, DirectChildRouting, PortFuture, ShutdownClock,
    ShutdownInstant, ShutdownSessionPersistence, SubagentLifecycleRepository, TurnCancellation,
};

pub fn identity(uuid: &str, generation: u64) -> DelegatedAgentIdentity {
    DelegatedAgentIdentity::new(uuid, LaunchGeneration::new(generation))
}

pub fn record(uuid: &str, generation: u64, parent: &str) -> LineageRecord {
    LineageRecord {
        identity: identity(uuid, generation),
        parent: AgentUuid::new(parent),
    }
}

/// root → A → B, A → C, root → D.
pub fn root_tree() -> LineageSnapshot {
    LineageSnapshot {
        owner: AgentUuid::new("root"),
        records: vec![
            record("A", 1, "root"),
            record("B", 1, "A"),
            record("C", 1, "A"),
            record("D", 1, "root"),
        ],
    }
}

pub struct FakeLifecycle {
    state: Mutex<HarnessLifecycleState>,
    lineage: Mutex<LineageSnapshot>,
    pub transitions: Mutex<Vec<HarnessLifecycleState>>,
}

impl FakeLifecycle {
    pub fn new(lineage: LineageSnapshot) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(HarnessLifecycleState::Accepting),
            lineage: Mutex::new(lineage),
            transitions: Mutex::new(Vec::new()),
        })
    }

    pub fn force(&self, state: HarnessLifecycleState) {
        *self.state.lock().unwrap() = state;
    }
}

impl SubagentLifecycleRepository for FakeLifecycle {
    fn lifecycle(&self) -> HarnessLifecycleState {
        *self.state.lock().unwrap()
    }

    fn set_lifecycle(&self, state: HarnessLifecycleState) {
        *self.state.lock().unwrap() = state;
        self.transitions.lock().unwrap().push(state);
    }

    fn lineage(&self) -> LineageSnapshot {
        self.lineage.lock().unwrap().clone()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutingCall {
    Shutdown(DelegatedAgentIdentity, ShutdownReason),
    Forward {
        via: DelegatedAgentIdentity,
        target: DelegatedAgentIdentity,
        remaining_depth: RoutingDepth,
    },
}

#[derive(Default)]
pub struct FakeRouting {
    pub calls: Mutex<Vec<RoutingCall>>,
    pub unreachable: Mutex<Vec<AgentUuid>>,
}

impl FakeRouting {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn outcome(&self, child: &AgentUuid) -> Result<(), ChildRoutingError> {
        if self.unreachable.lock().unwrap().contains(child) {
            Err(ChildRoutingError::Unreachable("socket closed".into()))
        } else {
            Ok(())
        }
    }

    pub fn calls(&self) -> Vec<RoutingCall> {
        self.calls.lock().unwrap().clone()
    }
}

impl DirectChildRouting for FakeRouting {
    fn shutdown_child<'a>(
        &'a self,
        child: &'a DelegatedAgentIdentity,
        reason: ShutdownReason,
    ) -> PortFuture<'a, Result<(), ChildRoutingError>> {
        self.calls
            .lock()
            .unwrap()
            .push(RoutingCall::Shutdown(child.clone(), reason));
        let outcome = self.outcome(&child.uuid);
        Box::pin(async move { outcome })
    }

    fn forward_termination<'a>(
        &'a self,
        via: &'a DelegatedAgentIdentity,
        target: &'a DelegatedAgentIdentity,
        remaining_depth: RoutingDepth,
    ) -> PortFuture<'a, Result<(), ChildRoutingError>> {
        self.calls.lock().unwrap().push(RoutingCall::Forward {
            via: via.clone(),
            target: target.clone(),
            remaining_depth,
        });
        let outcome = self.outcome(&via.uuid);
        Box::pin(async move { outcome })
    }
}

/// Cancellation that can be held open so a second caller must join.
pub struct FakeCancellation {
    pub calls: AtomicU64,
    pub in_flight: AtomicBool,
    pub gate: tokio::sync::Notify,
    pub hold: AtomicBool,
}

impl FakeCancellation {
    pub fn new(in_flight: bool) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicU64::new(0),
            in_flight: AtomicBool::new(in_flight),
            gate: tokio::sync::Notify::new(),
            hold: AtomicBool::new(false),
        })
    }

    pub fn holding() -> Arc<Self> {
        let fake = Self::new(true);
        fake.hold.store(true, Ordering::SeqCst);
        fake
    }
}

impl TurnCancellation for FakeCancellation {
    fn cancel_in_flight_turn(&self) -> PortFuture<'_, bool> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if self.hold.load(Ordering::SeqCst) {
                self.gate.notified().await;
            }
            self.in_flight.swap(false, Ordering::SeqCst)
        })
    }
}

#[derive(Default)]
pub struct FakePersistence {
    pub calls: Mutex<Vec<ShutdownReason>>,
    pub fail_with: Mutex<Option<String>>,
}

impl FakePersistence {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

impl ShutdownSessionPersistence for FakePersistence {
    fn persist_for_shutdown(&self, reason: ShutdownReason) -> PortFuture<'_, Result<(), String>> {
        self.calls.lock().unwrap().push(reason);
        let outcome = match self.fail_with.lock().unwrap().clone() {
            Some(detail) => Err(detail),
            None => Ok(()),
        };
        Box::pin(async move { outcome })
    }
}

pub struct FakeClock(pub AtomicU64);

impl FakeClock {
    pub fn at(ms: u64) -> Arc<Self> {
        Arc::new(Self(AtomicU64::new(ms)))
    }
}

impl ShutdownClock for FakeClock {
    fn now(&self) -> ShutdownInstant {
        ShutdownInstant(self.0.load(Ordering::SeqCst))
    }
}

#[derive(Default)]
pub struct FakeExit {
    pub signalled: Mutex<Vec<ShutdownReason>>,
}

impl FakeExit {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

impl CompositionExitReadiness for FakeExit {
    fn signal_exit_ready(&self, reason: ShutdownReason) -> PortFuture<'_, ()> {
        self.signalled.lock().unwrap().push(reason);
        Box::pin(async {})
    }
}
