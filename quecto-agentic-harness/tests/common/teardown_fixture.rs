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
    ChildRoutingError, CompositionExitReadiness, DirectChildRouting, PortFuture, ShutdownClock,
    ShutdownInstant, ShutdownSessionPersistence, SubagentLifecycleRepository, TurnCancellation,
};
use quecto::application::subagents::use_cases::{
    ExecuteHarnessShutdown, ExecuteHarnessShutdownPorts, HarnessShutdownTransaction,
    PrepareHarnessShutdown,
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

#[derive(Default)]
pub struct Routing {
    pub calls: Mutex<Vec<Call>>,
    pub unreachable: Mutex<Vec<AgentUuid>>,
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
        Box::pin(async move { outcome })
    }

    fn forward_termination<'a>(
        &'a self,
        via: &'a DelegatedAgentIdentity,
        target: &'a DelegatedAgentIdentity,
        remaining_depth: RoutingDepth,
    ) -> PortFuture<'a, Result<(), ChildRoutingError>> {
        self.calls.lock().unwrap().push(Call::Forward {
            via: via.clone(),
            target: target.clone(),
            remaining_depth,
        });
        let outcome = self.outcome(&via.uuid);
        Box::pin(async move { outcome })
    }
}

#[derive(Default)]
pub struct Cancellation {
    pub calls: AtomicUsize,
    pub in_flight: AtomicBool,
}

impl Cancellation {
    pub fn with_turn() -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            in_flight: AtomicBool::new(true),
        })
    }
}

impl TurnCancellation for Cancellation {
    fn cancel_in_flight_turn(&self) -> PortFuture<'_, bool> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move { self.in_flight.swap(false, Ordering::SeqCst) })
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

#[derive(Default)]
pub struct Exit {
    pub signalled: Mutex<Vec<ShutdownReason>>,
}

impl CompositionExitReadiness for Exit {
    fn signal_exit_ready(&self, reason: ShutdownReason) -> PortFuture<'_, ()> {
        self.signalled.lock().unwrap().push(reason);
        Box::pin(async {})
    }
}

/// A fully wired two-phase transaction over the fakes above.
pub struct Harness {
    pub lifecycle: Arc<Lifecycle>,
    pub routing: Arc<Routing>,
    pub cancellation: Arc<Cancellation>,
    pub persistence: Arc<Persistence>,
    pub clock: Arc<Clock>,
    pub exit: Arc<Exit>,
    pub prepare: PrepareHarnessShutdown,
    pub execute: ExecuteHarnessShutdown,
}

impl Harness {
    pub fn new(lineage: LineageSnapshot) -> Self {
        let lifecycle = Lifecycle::new(lineage);
        let routing = Routing::new();
        let cancellation = Cancellation::with_turn();
        let persistence = Arc::new(Persistence::default());
        let clock = Arc::new(Clock(AtomicU64::new(1_000)));
        let exit = Arc::new(Exit::default());
        let transaction = HarnessShutdownTransaction::new(lifecycle.clone(), clock.clone());
        let prepare = PrepareHarnessShutdown::new(transaction.clone());
        let execute = ExecuteHarnessShutdown::new(
            transaction,
            ExecuteHarnessShutdownPorts {
                routing: routing.clone(),
                cancellation: cancellation.clone(),
                persistence: persistence.clone(),
                exit: exit.clone(),
            },
        );
        Self {
            lifecycle,
            routing,
            cancellation,
            persistence,
            clock,
            exit,
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
