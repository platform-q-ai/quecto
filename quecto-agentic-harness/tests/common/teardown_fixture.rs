//! Shared port fakes for the subagent teardown contract suites (#1934).
//! Built only from the public crate surface, so they prove the ports are
//! implementable from outside the application layer.
#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use quecto::application::subagents::dto::{
    PrepareShutdownRequest, PreparedShutdown, ShutdownTrigger,
};
use quecto::application::subagents::ports::{
    ChildRoutingError, Compensated, CompensationObservation, CompositionExitReadiness,
    ConclusionBudget, DelegatedAgentRegistry, DirectChildRouting, ExitReadiness,
    OwnedChildTermination, PortFuture, ProtocolAttempt, ResolutionError, ShutdownClock,
    ShutdownInstant, ShutdownRun, ShutdownRunSpawner, ShutdownSessionPersistence,
    StoppingClaimError, SubagentLifecycleRepository, TeardownCompensation, TerminalClaim,
    TerminationCause, TerminationConclusion, TurnCancellation,
};
use quecto::application::subagents::use_cases::{
    ExecuteHarnessShutdown, ExecuteHarnessShutdownPorts, HarnessShutdownTransaction,
    PrepareHarnessShutdown, TerminateAllDelegatedAgents, TerminateAllDelegatedAgentsPorts,
};
use quecto::domain::ids::AgentUuid;
use quecto::domain::subagent_teardown::{
    DelegatedAgentIdentity, HarnessLifecycleState, LaunchGeneration, LineageRecord,
    LineageSnapshot, RoutingDepth, ShutdownReason,
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

pub struct Lifecycle {
    state: Mutex<HarnessLifecycleState>,
    lineage: LineageSnapshot,
    pub transitions: Mutex<Vec<HarnessLifecycleState>>,
}

impl Lifecycle {
    pub fn new(lineage: LineageSnapshot) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(HarnessLifecycleState::Accepting),
            lineage,
            transitions: Mutex::new(Vec::new()),
        })
    }

    /// Put the repository in a state without going through the domain, to
    /// stage scenarios such as "already terminated".
    pub fn force(&self, state: HarnessLifecycleState) {
        *self.state.lock().unwrap() = state;
    }
}

impl SubagentLifecycleRepository for Lifecycle {
    fn lifecycle(&self) -> HarnessLifecycleState {
        *self.state.lock().unwrap()
    }

    fn set_lifecycle(&self, state: HarnessLifecycleState) {
        *self.state.lock().unwrap() = state;
        self.transitions.lock().unwrap().push(state);
    }

    fn lineage(&self) -> LineageSnapshot {
        self.lineage.clone()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    Shutdown(DelegatedAgentIdentity, ShutdownReason),
    Forward {
        via: DelegatedAgentIdentity,
        target: DelegatedAgentIdentity,
        remaining_depth: RoutingDepth,
    },
}

/// Routing that can hold one child's shutdown open so a run can be
/// interrupted mid-loop.
#[derive(Default)]
pub struct Routing {
    pub calls: Mutex<Vec<Call>>,
    pub unreachable: Mutex<Vec<AgentUuid>>,
    pub hold_child: Mutex<Option<AgentUuid>>,
    pub gate: tokio::sync::Notify,
}

impl Routing {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    fn outcome(&self, child: &AgentUuid) -> Result<(), ChildRoutingError> {
        if self.unreachable.lock().unwrap().contains(child) {
            Err(ChildRoutingError::Unreachable("peer gone".into()))
        } else {
            Ok(())
        }
    }
}

impl DirectChildRouting for Routing {
    fn shutdown_child<'a>(
        &'a self,
        child: &'a DelegatedAgentIdentity,
        reason: ShutdownReason,
    ) -> PortFuture<'a, Result<(), ChildRoutingError>> {
        self.calls
            .lock()
            .unwrap()
            .push(Call::Shutdown(child.clone(), reason));
        let outcome = self.outcome(&child.uuid);
        let held = self.hold_child.lock().unwrap().as_ref() == Some(&child.uuid);
        Box::pin(async move {
            if held {
                self.gate.notified().await;
            }
            outcome
        })
    }

    fn forward_termination<'a>(
        &'a self,
        via: &'a DelegatedAgentIdentity,
        target: &'a DelegatedAgentIdentity,
        remaining_depth: RoutingDepth,
    ) -> PortFuture<
        'a,
        Result<Option<quecto::application::subagents::dto::TerminationResult>, ChildRoutingError>,
    > {
        self.calls.lock().unwrap().push(Call::Forward {
            via: via.clone(),
            target: target.clone(),
            remaining_depth,
        });
        let outcome = self.outcome(&via.uuid).map(|()| None);
        Box::pin(async move { outcome })
    }
}

/// Cancellation that can be held open (`hold`) so a second caller must
/// join, or a runner can be cancelled while parked inside it.
pub struct Cancellation {
    pub calls: AtomicUsize,
    pub in_flight: AtomicBool,
    pub hold: AtomicBool,
    pub gate: tokio::sync::Notify,
}

impl Cancellation {
    pub fn with_turn() -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            in_flight: AtomicBool::new(true),
            hold: AtomicBool::new(false),
            gate: tokio::sync::Notify::new(),
        })
    }
}

impl TurnCancellation for Cancellation {
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
pub struct Persistence {
    pub calls: Mutex<Vec<ShutdownReason>>,
    pub fail_with: Mutex<Option<String>>,
}

impl ShutdownSessionPersistence for Persistence {
    fn persist_for_shutdown(&self, reason: ShutdownReason) -> PortFuture<'_, Result<(), String>> {
        self.calls.lock().unwrap().push(reason);
        let outcome = self.fail_with.lock().unwrap().clone().map_or(Ok(()), Err);
        Box::pin(async move { outcome })
    }
}

pub struct Clock(pub AtomicU64);

impl ShutdownClock for Clock {
    fn now(&self) -> ShutdownInstant {
        ShutdownInstant(self.0.load(Ordering::SeqCst))
    }
}

/// Exit readiness that can be held open so a run can be dropped between
/// persistence and the exit signal.
#[derive(Default)]
pub struct Exit {
    pub signalled: Mutex<Vec<ExitReadiness>>,
    pub attempts: AtomicUsize,
    pub hold: AtomicBool,
    pub gate: tokio::sync::Notify,
}

impl Exit {
    pub fn signalled(&self) -> Vec<ExitReadiness> {
        self.signalled.lock().unwrap().clone()
    }
}

impl CompositionExitReadiness for Exit {
    fn signal_exit_ready(&self, readiness: ExitReadiness) -> PortFuture<'_, ()> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if self.hold.load(Ordering::SeqCst) {
                self.gate.notified().await;
            }
            self.signalled.lock().unwrap().push(readiness);
        })
    }
}

/// Spawner over the ambient tokio runtime that keeps run handles so a test
/// can abort a detached run, and can drop the next N runs unpolled.
#[derive(Default)]
pub struct Spawner {
    pub handles: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    pub drop_next: AtomicUsize,
    pub spawned: AtomicUsize,
}

impl Spawner {
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

impl ShutdownRunSpawner for Spawner {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowPhase {
    Live,
    Stopping(TerminationCause),
    Compensating,
    Compensated,
}

/// In-memory rows of one harness with the per-row claim ladder the fleet
/// teardown relies on (#1938); doubles as the compensation and, with a
/// scripted conclusion, the owned-handle fallback.
#[derive(Default)]
pub struct Rows {
    pub phases: Mutex<std::collections::BTreeMap<String, (DelegatedAgentIdentity, RowPhase)>>,
    pub changed: tokio::sync::Notify,
    pub conclusion: Mutex<Option<TerminationConclusion>>,
    /// Per-child overrides of `conclusion`.
    pub conclusion_for: Mutex<std::collections::BTreeMap<String, TerminationConclusion>>,
    pub compensated: Mutex<Vec<(DelegatedAgentIdentity, TerminationCause)>>,
    pub concluded: Mutex<Vec<(DelegatedAgentIdentity, ProtocolAttempt)>>,
}

impl Rows {
    /// One live row per direct child of `lineage`.
    pub fn for_lineage(lineage: &LineageSnapshot) -> Arc<Self> {
        let rows = Arc::new(Self::default());
        for child in lineage.direct_children() {
            rows.phases.lock().unwrap().insert(
                child.uuid.as_str().to_owned(),
                (child.clone(), RowPhase::Live),
            );
        }
        rows
    }

    pub fn phase(&self, uuid: &str) -> Option<RowPhase> {
        self.phases
            .lock()
            .unwrap()
            .get(uuid)
            .map(|(_, phase)| *phase)
    }

    pub fn set(&self, uuid: &str, phase: RowPhase) {
        if let Some(row) = self.phases.lock().unwrap().get_mut(uuid) {
            row.1 = phase;
        }
        self.changed.notify_waiters();
    }
}

impl DelegatedAgentRegistry for Rows {
    fn resolve(&self, reference: &str) -> Result<DelegatedAgentIdentity, ResolutionError> {
        self.phases
            .lock()
            .unwrap()
            .get(reference)
            .map(|(identity, _)| identity.clone())
            .ok_or(ResolutionError::Unknown)
    }

    fn claim_stopping(
        &self,
        target: &DelegatedAgentIdentity,
        cause: TerminationCause,
    ) -> Result<(), StoppingClaimError> {
        let mut phases = self.phases.lock().unwrap();
        let row = phases
            .get_mut(target.uuid.as_str())
            .filter(|(identity, _)| identity == target)
            .ok_or(StoppingClaimError::Unknown)?;
        match row.1 {
            RowPhase::Live => {
                row.1 = RowPhase::Stopping(cause);
                Ok(())
            }
            RowPhase::Stopping(_) => Err(StoppingClaimError::AlreadyStopping),
            RowPhase::Compensating | RowPhase::Compensated => Err(StoppingClaimError::Exited),
        }
    }

    fn release_stopping(&self, target: &DelegatedAgentIdentity) {
        if matches!(
            self.phase(target.uuid.as_str()),
            Some(RowPhase::Stopping(_))
        ) {
            self.set(target.uuid.as_str(), RowPhase::Live);
        }
    }

    fn claim_terminal(&self, target: &DelegatedAgentIdentity) -> TerminalClaim {
        let mut phases = self.phases.lock().unwrap();
        let Some(row) = phases.get_mut(target.uuid.as_str()) else {
            return TerminalClaim::AlreadyClaimed;
        };
        match row.1 {
            RowPhase::Live | RowPhase::Stopping(_) => {
                row.1 = RowPhase::Compensating;
                TerminalClaim::Claimed
            }
            RowPhase::Compensating | RowPhase::Compensated => TerminalClaim::AlreadyClaimed,
        }
    }

    fn holds_process(&self, _target: &DelegatedAgentIdentity) -> bool {
        false
    }

    fn terminal_claimed(&self, target: &DelegatedAgentIdentity) -> bool {
        matches!(
            self.phase(target.uuid.as_str()),
            Some(RowPhase::Compensating | RowPhase::Compensated)
        )
    }

    fn await_compensated<'a>(
        &'a self,
        target: &'a DelegatedAgentIdentity,
    ) -> PortFuture<'a, CompensationObservation> {
        Box::pin(async move {
            loop {
                let notified = self.changed.notified();
                match self.phase(target.uuid.as_str()) {
                    None => return CompensationObservation::Unknown,
                    Some(RowPhase::Compensated) => return CompensationObservation::Compensated,
                    Some(_) => {}
                }
                notified.await;
            }
        })
    }
}

impl OwnedChildTermination for Rows {
    fn conclude<'a>(
        &'a self,
        child: &'a DelegatedAgentIdentity,
        attempt: ProtocolAttempt,
        _budget: ConclusionBudget,
    ) -> PortFuture<'a, TerminationConclusion> {
        self.concluded
            .lock()
            .unwrap()
            .push((child.clone(), attempt));
        let conclusion = self
            .conclusion_for
            .lock()
            .unwrap()
            .get(child.uuid.as_str())
            .cloned()
            .or_else(|| self.conclusion.lock().unwrap().clone())
            .unwrap_or(TerminationConclusion::ExitedAfterProtocol);
        Box::pin(async move { conclusion })
    }
}

impl TeardownCompensation for Rows {
    fn compensate<'a>(
        &'a self,
        target: &'a DelegatedAgentIdentity,
        cause: TerminationCause,
    ) -> PortFuture<'a, Compensated> {
        self.compensated
            .lock()
            .unwrap()
            .push((target.clone(), cause));
        self.set(target.uuid.as_str(), RowPhase::Compensated);
        Box::pin(async move {
            Compensated {
                removed: vec![target.uuid.clone()],
            }
        })
    }

    fn prune_terminal_rows(&self) -> PortFuture<'_, Vec<AgentUuid>> {
        Box::pin(async move {
            let mut phases = self.phases.lock().unwrap();
            let pruned: Vec<AgentUuid> = phases
                .values()
                .filter(|(_, phase)| *phase == RowPhase::Compensated)
                .map(|(identity, _)| identity.uuid.clone())
                .collect();
            phases.retain(|_, (_, phase)| *phase != RowPhase::Compensated);
            pruned
        })
    }
}

/// A fully wired two-phase transaction over the fakes above, with the
/// fleet teardown as its children step.
pub struct Harness {
    pub lifecycle: Arc<Lifecycle>,
    pub routing: Arc<Routing>,
    pub rows: Arc<Rows>,
    pub fleet: Arc<TerminateAllDelegatedAgents>,
    pub cancellation: Arc<Cancellation>,
    pub persistence: Arc<Persistence>,
    pub clock: Arc<Clock>,
    pub exit: Arc<Exit>,
    pub spawner: Arc<Spawner>,
    pub prepare: PrepareHarnessShutdown,
    pub execute: Arc<ExecuteHarnessShutdown>,
}

impl Harness {
    pub fn new(lineage: LineageSnapshot) -> Self {
        let rows = Rows::for_lineage(&lineage);
        let lifecycle = Lifecycle::new(lineage);
        let routing = Routing::new();
        let cancellation = Cancellation::with_turn();
        let persistence = Arc::new(Persistence::default());
        let clock = Arc::new(Clock(AtomicU64::new(1_000)));
        let exit = Arc::new(Exit::default());
        let spawner = Arc::new(Spawner::default());
        let fleet = Arc::new(TerminateAllDelegatedAgents::new(
            TerminateAllDelegatedAgentsPorts {
                lifecycle: lifecycle.clone(),
                registry: rows.clone(),
                routing: routing.clone(),
                termination: rows.clone(),
                compensation: rows.clone(),
                spawner: Arc::new(Spawner::default()),
            },
        ));
        let transaction = HarnessShutdownTransaction::new(lifecycle.clone(), clock.clone());
        let prepare = PrepareHarnessShutdown::new(transaction.clone());
        let execute = Arc::new(ExecuteHarnessShutdown::new(
            transaction,
            ExecuteHarnessShutdownPorts {
                children: fleet.clone(),
                cancellation: cancellation.clone(),
                persistence: persistence.clone(),
                exit: exit.clone(),
                spawner: spawner.clone(),
            },
        ));
        Self {
            lifecycle,
            routing,
            rows,
            fleet,
            cancellation,
            persistence,
            clock,
            exit,
            spawner,
            prepare,
            execute,
        }
    }

    pub fn prepared(&self, reason: ShutdownReason) -> PreparedShutdown {
        self.prepare
            .execute(PrepareShutdownRequest {
                reason,
                trigger: ShutdownTrigger::ProtocolCommand,
            })
            .unwrap()
    }
}
