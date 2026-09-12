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
    ShutdownInstant, ShutdownRun, ShutdownRunSpawner, ShutdownSessionPersistence,
    SubagentLifecycleRepository, TurnCancellation,
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
    /// Shared ordering trace: the first teardown effect records itself here
    /// so a test can prove it happened after (never before) the ACK flush.
    pub trace: Mutex<Option<Arc<Mutex<Vec<String>>>>>,
}

impl FakeCancellation {
    pub fn new(in_flight: bool) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicU64::new(0),
            in_flight: AtomicBool::new(in_flight),
            gate: tokio::sync::Notify::new(),
            hold: AtomicBool::new(false),
            trace: Mutex::new(None),
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
        if let Some(trace) = self.trace.lock().unwrap().as_ref() {
            trace.lock().unwrap().push("execute-started".into());
        }
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

/// Exit readiness that can be held open so a run can be dropped between
/// persistence and the exit signal.
#[derive(Default)]
pub struct FakeExit {
    pub signalled: Mutex<Vec<ShutdownReason>>,
    pub attempts: AtomicU64,
    pub hold: AtomicBool,
    pub gate: tokio::sync::Notify,
}

impl FakeExit {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

impl CompositionExitReadiness for FakeExit {
    fn signal_exit_ready(&self, reason: ShutdownReason) -> PortFuture<'_, ()> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if self.hold.load(Ordering::SeqCst) {
                self.gate.notified().await;
            }
            self.signalled.lock().unwrap().push(reason);
        })
    }
}

/// Spawner over the test runtime that keeps every run's handle so a test
/// can abort a detached run (simulating its runtime dropping it), or drop
/// runs outright to simulate a spawner with no runtime left.
#[derive(Default)]
pub struct FakeSpawner {
    pub handles: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    /// Drop (never run) this many of the next spawned runs.
    pub drop_next: AtomicU64,
    pub spawned: AtomicU64,
}

impl FakeSpawner {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// The spawned task may start running before its handle is recorded,
    /// so wait for the record rather than racing it.
    async fn take_latest(&self) -> tokio::task::JoinHandle<()> {
        loop {
            if let Some(handle) = self.handles.lock().unwrap().pop() {
                return handle;
            }
            tokio::task::yield_now().await;
        }
    }

    pub async fn abort_latest(&self) {
        self.take_latest().await.abort();
    }

    pub async fn latest_finished(&self) {
        let _ = self.take_latest().await.await;
    }
}

impl ShutdownRunSpawner for FakeSpawner {
    fn spawn_shutdown_run(&self, run: ShutdownRun) {
        self.spawned.fetch_add(1, Ordering::SeqCst);
        if self.drop_next.load(Ordering::SeqCst) > 0 {
            self.drop_next.fetch_sub(1, Ordering::SeqCst);
            drop(run);
            return;
        }
        self.handles.lock().unwrap().push(tokio::spawn(run));
    }
}
