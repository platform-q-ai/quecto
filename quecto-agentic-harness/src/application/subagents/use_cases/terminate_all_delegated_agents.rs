//! Fleet teardown of every direct child (#1938, epic #1929).
//!
//! One run settles the whole fleet: every direct child of this harness is
//! claimed stopping, asked to shut down over its one edge, concluded through
//! the owned-handle fallback (protocol first, a signal only after a negative
//! outcome or an exit timeout), and compensated exactly once — or the path
//! that already ended it (reaper, monitor, operator kill) is joined. Children
//! settle concurrently under a bound. A child whose end cannot be observed
//! within its budget is reported *unsettled* with its stopping claim lifted:
//! the caller decides whether to abort (a session transition) or continue
//! (a process exit); ownership is never silently lost.
//!
//! The run is detached onto the capability's [`ShutdownRunSpawner`] and every
//! caller merely joins it: a delete-all, a termination signal and a session
//! transition arriving together produce one run and one outcome, and a
//! caller dropped mid-run (its connection went away) never abandons claims
//! it took. Once the fleet has settled, terminal rows (exited tombstones)
//! are pruned from the roster so a persisted roster never carries a live
//! operational child.
//!
//! This use case never freezes spawn admission and never cancels a turn:
//! those belong to the harness shutdown that drives it. A spawn that
//! registers while the run is in flight is picked up by a further pass
//! (bounded), so an operator's delete-all leaves no direct child behind.
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};

use crate::domain::ids::AgentUuid;
use crate::domain::subagent_teardown::{DelegatedAgentIdentity, ShutdownReason};

use super::super::dto::{
    FleetChildResult, FleetTeardownError, FleetTeardownOutcome, SettledChild,
    TerminateAllDelegatedAgentsRequest,
};
use super::super::ports::{
    CompensationObservation, ConclusionBudget, DelegatedAgentRegistry, DirectChildRouting,
    OwnedChildTermination, ProtocolAttempt, ShutdownRunSpawner, StoppingClaimError,
    SubagentLifecycleRepository, TeardownCompensation, TerminalClaim, TerminationCause,
    TerminationConclusion,
};
use super::bounded_settlement::{BoundedSettlement, Settlement};

/// Children settled concurrently by default: enough to keep a typical fleet
/// prompt, small enough that a large one never opens every edge at once.
pub const DEFAULT_SETTLEMENT_BOUND: usize = 8;

/// Passes over the lineage one run makes at most: the first settles what
/// was there, later ones catch children registered while it ran.
const MAX_PASSES: usize = 3;

type Outcome = Result<FleetTeardownOutcome, FleetTeardownError>;

pub struct TerminateAllDelegatedAgentsPorts {
    pub lifecycle: Arc<dyn SubagentLifecycleRepository>,
    pub registry: Arc<dyn DelegatedAgentRegistry>,
    pub routing: Arc<dyn DirectChildRouting>,
    pub termination: Arc<dyn OwnedChildTermination>,
    pub compensation: Arc<dyn TeardownCompensation>,
    pub spawner: Arc<dyn ShutdownRunSpawner>,
}

pub struct TerminateAllDelegatedAgents {
    inner: Arc<Inner>,
}

struct Inner {
    ports: TerminateAllDelegatedAgentsPorts,
    bound: usize,
    in_flight: Mutex<Option<Arc<Run>>>,
}

/// One detached run and everyone waiting on it.
struct Run {
    outcome: Mutex<Option<Outcome>>,
    wakers: Mutex<Vec<Waker>>,
}

impl Run {
    fn complete(&self, outcome: Outcome) {
        *lock(&self.outcome) = Some(outcome);
        for waker in lock(&self.wakers).drain(..) {
            waker.wake();
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// What settling one child established.
enum ChildSettlement {
    Done(SettledChild),
    Unsettled(AgentUuid, String),
    /// The row was gone (uncommitted or unknown) before anything ran.
    Gone(AgentUuid),
}

impl TerminateAllDelegatedAgents {
    pub fn new(ports: TerminateAllDelegatedAgentsPorts) -> Self {
        Self::with_bound(ports, DEFAULT_SETTLEMENT_BOUND)
    }

    pub fn with_bound(ports: TerminateAllDelegatedAgentsPorts, bound: usize) -> Self {
        assert!(
            bound >= 1,
            "the settlement bound must admit at least one child"
        );
        Self {
            inner: Arc::new(Inner {
                ports,
                bound,
                in_flight: Mutex::new(None),
            }),
        }
    }

    /// Whether a run is in flight right now.
    pub fn in_flight(&self) -> bool {
        lock(&self.inner.in_flight).is_some()
    }

    /// Number of callers parked on the in-flight run. Test probe so a test
    /// can make a joiner's registration observable before it releases
    /// whatever the run is waiting on.
    #[cfg(any(test, feature = "test-support"))]
    pub fn waiting_joiners(&self) -> usize {
        lock(&self.inner.in_flight)
            .as_ref()
            .map_or(0, |run| lock(&run.wakers).len())
    }

    /// Start the fleet teardown, or join the one already running, and wait
    /// for its outcome. The run itself is detached: dropping this future
    /// never stops it.
    pub async fn execute(&self, request: TerminateAllDelegatedAgentsRequest) -> Outcome {
        // The slot is claimed under the lock; the run is spawned outside it,
        // because a spawner that drops the run at once settles it (and
        // clears the slot) synchronously.
        let (run, guard) = {
            let mut in_flight = lock(&self.inner.in_flight);
            match in_flight.as_ref() {
                Some(run) => (run.clone(), None),
                None => {
                    let run = Arc::new(Run {
                        outcome: Mutex::new(None),
                        wakers: Mutex::new(Vec::new()),
                    });
                    *in_flight = Some(run.clone());
                    let guard = RunGuard {
                        inner: self.inner.clone(),
                        run: run.clone(),
                        armed: true,
                    };
                    (run, Some(guard))
                }
            }
        };
        let joined = guard.is_none();
        if let Some(guard) = guard {
            self.inner
                .ports
                .spawner
                .spawn_shutdown_run(Box::pin(drive(guard, request.reason)));
        }
        let outcome = JoinRun { run: &run }.await;
        outcome.map(|mut outcome| {
            outcome.joined = joined;
            outcome
        })
    }
}

/// The detached run: settle every direct child in bounded concurrent
/// batches, pass again for late registrations, then prune the tombstones.
async fn drive(guard: RunGuard, reason: ShutdownReason) {
    let inner = guard.inner.clone();
    let mut settled: Vec<SettledChild> = Vec::new();
    let mut unsettled: Vec<(AgentUuid, String)> = Vec::new();
    let mut gone: Vec<AgentUuid> = Vec::new();
    for _pass in 0..MAX_PASSES {
        let children: Vec<DelegatedAgentIdentity> = inner
            .ports
            .lifecycle
            .lineage()
            .direct_children()
            .filter(|child| {
                !settled.iter().any(|done| &done.child == *child)
                    && !unsettled.iter().any(|(uuid, _)| uuid == &child.uuid)
                    && !gone.contains(&child.uuid)
            })
            .cloned()
            .collect();
        if children.is_empty() {
            break;
        }
        let batch: Vec<Settlement<ChildSettlement>> = children
            .into_iter()
            .map(|child| {
                let inner = inner.clone();
                Box::pin(async move { inner.settle_child(child, reason).await })
                    as Settlement<ChildSettlement>
            })
            .collect();
        for outcome in BoundedSettlement::new(inner.bound, batch).await {
            match outcome {
                ChildSettlement::Done(child) => settled.push(child),
                ChildSettlement::Unsettled(uuid, detail) => unsettled.push((uuid, detail)),
                ChildSettlement::Gone(uuid) => gone.push(uuid),
            }
        }
    }
    settled.sort_by(|a, b| a.child.uuid.cmp(&b.child.uuid));
    unsettled.sort_by(|a, b| a.0.cmp(&b.0));
    let pruned = inner.ports.compensation.prune_terminal_rows().await;
    guard.finish(Ok(FleetTeardownOutcome {
        reason,
        joined: false,
        settled,
        unsettled,
        pruned,
    }));
}

impl Inner {
    /// Claim, ask, conclude, compensate — or join whoever already did.
    async fn settle_child(
        &self,
        child: DelegatedAgentIdentity,
        reason: ShutdownReason,
    ) -> ChildSettlement {
        match self
            .ports
            .registry
            .claim_stopping(&child, TerminationCause::FleetTeardown)
        {
            Ok(()) => {}
            Err(StoppingClaimError::AlreadyStopping) => {
                // A kill, rollback or another teardown owns this child: its
                // compensation is the only truthful end to wait for.
                return self.join_child(child, FleetChildResult::Joined).await;
            }
            Err(StoppingClaimError::Unknown | StoppingClaimError::Exited) => {
                return ChildSettlement::Gone(child.uuid);
            }
        }
        let attempt = match self.ports.routing.shutdown_child(&child, reason).await {
            Ok(()) => ProtocolAttempt::Acknowledged,
            Err(error) => ProtocolAttempt::Negative(error.to_string()),
        };
        let acknowledged = matches!(attempt, ProtocolAttempt::Acknowledged);
        let result = match self
            .ports
            .termination
            .conclude(&child, attempt, ConclusionBudget::Standard)
            .await
        {
            TerminationConclusion::ExitedAfterProtocol => FleetChildResult::Graceful,
            TerminationConclusion::ExitedAfterFallback => FleetChildResult::Fallback,
            TerminationConclusion::AlreadyExited => FleetChildResult::AlreadyExited,
            TerminationConclusion::StillRunning(detail) => {
                // Even the fallback did not end it: the row keeps its record
                // and the claim is lifted so a later trigger may try again.
                self.ports.registry.release_stopping(&child);
                return ChildSettlement::Unsettled(child.uuid, detail);
            }
            TerminationConclusion::NoRetainedHandle => {
                self.conclude_unowned(&child, acknowledged).await
            }
        };
        self.compensate_or_join(child, result).await
    }

    /// A child this harness holds no process for: its end is observed only
    /// through its row (monitor EOF, reported prune, reaper of an earlier
    /// handle). An acknowledged child gets the bound to exit on its own;
    /// past it — or after a negative attempt — the compensation runs
    /// without an observed exit, which finalizes its environment.
    async fn conclude_unowned(
        &self,
        child: &DelegatedAgentIdentity,
        acknowledged: bool,
    ) -> FleetChildResult {
        if self.ports.registry.terminal_claimed(child) {
            return FleetChildResult::AlreadyExited;
        }
        if !acknowledged {
            return FleetChildResult::Unobserved;
        }
        match self.ports.registry.await_compensated(child).await {
            CompensationObservation::Compensated => FleetChildResult::Graceful,
            CompensationObservation::TimedOut | CompensationObservation::Unknown => {
                FleetChildResult::Unobserved
            }
        }
    }

    async fn compensate_or_join(
        &self,
        child: DelegatedAgentIdentity,
        result: FleetChildResult,
    ) -> ChildSettlement {
        match self.ports.registry.claim_terminal(&child) {
            TerminalClaim::Claimed => {
                self.ports
                    .compensation
                    .compensate(&child, TerminationCause::FleetTeardown)
                    .await;
                ChildSettlement::Done(SettledChild { child, result })
            }
            TerminalClaim::AlreadyClaimed => self.join_child(child, result).await,
        }
    }

    async fn join_child(
        &self,
        child: DelegatedAgentIdentity,
        result: FleetChildResult,
    ) -> ChildSettlement {
        match self.ports.registry.await_compensated(&child).await {
            CompensationObservation::Compensated => {
                ChildSettlement::Done(SettledChild { child, result })
            }
            CompensationObservation::Unknown => ChildSettlement::Gone(child.uuid),
            CompensationObservation::TimedOut => ChildSettlement::Unsettled(
                child.uuid,
                "a termination already in flight did not settle within the bound".into(),
            ),
        }
    }
}

/// Completes the run, or — if the detached run is dropped by its runtime
/// before it finishes — reports the interruption to every joiner and clears
/// the in-flight slot so the next trigger starts afresh.
struct RunGuard {
    inner: Arc<Inner>,
    run: Arc<Run>,
    armed: bool,
}

impl RunGuard {
    fn finish(mut self, outcome: Outcome) {
        self.armed = false;
        self.settle(outcome);
    }

    fn settle(&self, outcome: Outcome) {
        {
            let mut in_flight = lock(&self.inner.in_flight);
            if in_flight
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &self.run))
            {
                *in_flight = None;
            }
        }
        self.run.complete(outcome);
    }
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        if self.armed {
            self.settle(Err(FleetTeardownError::Interrupted));
        }
    }
}

struct JoinRun<'a> {
    run: &'a Run,
}

impl Future for JoinRun<'_> {
    type Output = Outcome;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // The outcome lock is taken before the waker lock and held across
        // the registration, so a completion cannot slip between the check
        // and the registration.
        let outcome = lock(&self.run.outcome);
        if let Some(outcome) = outcome.as_ref() {
            return Poll::Ready(outcome.clone());
        }
        let mut wakers = lock(&self.run.wakers);
        if !wakers.iter().any(|w| w.will_wake(cx.waker())) {
            wakers.push(cx.waker().clone());
        }
        Poll::Pending
    }
}

#[cfg(test)]
#[path = "terminate_all_delegated_agents_tests.rs"]
mod tests;
