//! Two-phase, idempotent harness shutdown (#1934).
//!
//! `Prepare` admits a shutdown and freezes new prompt/spawn work, handing back
//! an opaque per-holder token. The interface then writes and flushes the
//! correlated ACK; only after that does it hand the token to `Execute`. If the
//! ACK cannot be written the interface releases its holder instead, so a
//! parent that never received an ACK never sees the child vanish silently: it
//! can tell a graceful exit (ACK then EOF) from a loss (EOF with no ACK).
//!
//! Every trigger — protocol command, bound-parent connection closure, OS
//! signal — goes through the same transaction. The first one admits; later
//! ones join as further holders and observe the single outcome. The freeze
//! lifts only when every holder has released and nothing ran.
//!
//! Once admitted for execution the run belongs to the transaction, not to the
//! caller: `Execute` hands it to a [`ShutdownRunSpawner`] and merely joins it,
//! so dropping the caller's future cannot abandon a shutdown whose ACK is on
//! the wire. Progress is recorded step by step; if the spawned run is ever
//! dropped, the next `Execute` resumes at the first incomplete step and no
//! effect runs twice. No ACK is written here and no wire type is named here.
use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};

use crate::domain::ids::AgentUuid;
use crate::domain::subagent_teardown::{HarnessLifecycleState, ShutdownReason};

use super::super::dto::{
    HarnessShutdownError, PersistenceOutcome, PrepareShutdownRequest, PreparedShutdown,
    ReleaseOutcome, ShutdownOutcome, ShutdownToken, ShutdownTrigger,
};
use super::super::ports::{
    CompositionExitReadiness, DirectChildRouting, ShutdownClock, ShutdownRunSpawner,
    ShutdownSessionPersistence, SubagentLifecycleRepository, TurnCancellation,
};

type Outcome = Result<ShutdownOutcome, HarnessShutdownError>;

/// Direct children addressed by the children step: shut down, and failed
/// with the routing error rendered.
type ChildrenOutcome = (Vec<AgentUuid>, Vec<(AgentUuid, String)>);

/// Steps of the common teardown that have already produced their effect.
/// A re-driven run skips every recorded step.
#[derive(Debug, Clone, Default)]
struct Progress {
    turn_cancelled: Option<bool>,
    children: Option<ChildrenOutcome>,
    persistence: Option<PersistenceOutcome>,
    exit_signalled: bool,
}

/// One admitted shutdown and everyone holding it.
#[derive(Debug, Clone)]
struct Admission {
    id: u64,
    reason: ShutdownReason,
    triggers: Vec<ShutdownTrigger>,
    next_holder: u64,
    /// Holders that have not released.
    holders: BTreeSet<u64>,
    /// Holders that released; their tokens are spent.
    released: BTreeSet<u64>,
    progress: Progress,
}

impl Admission {
    fn join(&mut self, trigger: ShutdownTrigger) -> ShutdownToken {
        let holder = self.next_holder;
        self.next_holder += 1;
        self.holders.insert(holder);
        self.triggers.push(trigger);
        ShutdownToken::mint(self.id, holder)
    }

    /// The token must name this admission and a holder that has not spent it.
    fn validate(&self, token: &ShutdownToken) -> Result<(), HarnessShutdownError> {
        if token.admission() != self.id {
            return Err(HarnessShutdownError::UnknownToken);
        }
        if self.holders.contains(&token.holder()) {
            return Ok(());
        }
        if self.released.contains(&token.holder()) {
            return Err(HarnessShutdownError::TokenReleased);
        }
        Err(HarnessShutdownError::UnknownToken)
    }
}

enum Phase {
    Idle,
    Prepared(Admission),
    Executing {
        admission: Admission,
        wakers: Vec<Waker>,
    },
    Completed {
        admission: Admission,
        outcome: Outcome,
    },
}

impl Phase {
    fn admission(&self) -> Option<&Admission> {
        match self {
            Self::Idle => None,
            Self::Prepared(admission)
            | Self::Executing { admission, .. }
            | Self::Completed { admission, .. } => Some(admission),
        }
    }

    fn admission_mut(&mut self) -> Option<&mut Admission> {
        match self {
            Self::Idle => None,
            Self::Prepared(admission)
            | Self::Executing { admission, .. }
            | Self::Completed { admission, .. } => Some(admission),
        }
    }
}

/// Process-wide admission serial so two transactions never hand out equal
/// tokens.
static ADMISSIONS: AtomicU64 = AtomicU64::new(0);

/// Shared admission state behind both phases. One instance per harness.
pub struct HarnessShutdownTransaction {
    phase: Mutex<Phase>,
    lifecycle: Arc<dyn SubagentLifecycleRepository>,
    clock: Arc<dyn ShutdownClock>,
}

impl HarnessShutdownTransaction {
    pub fn new(
        lifecycle: Arc<dyn SubagentLifecycleRepository>,
        clock: Arc<dyn ShutdownClock>,
    ) -> Arc<Self> {
        Arc::new(Self {
            phase: Mutex::new(Phase::Idle),
            lifecycle,
            clock,
        })
    }

    fn lock(&self) -> MutexGuard<'_, Phase> {
        self.phase
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn mint_admission_id(&self) -> u64 {
        // Opaque: a process-wide serial mixed with the clock so unrelated
        // admissions never mint equal tokens, even on a frozen clock.
        let serial = ADMISSIONS.fetch_add(1, Ordering::SeqCst).wrapping_add(1);
        serial.rotate_left(32) ^ self.clock.now().0
    }

    /// Whether new prompt/spawn work may still be admitted.
    pub fn accepts_new_work(&self) -> bool {
        self.lifecycle.lifecycle().accepts_new_work()
    }

    fn with_progress<T>(&self, admission_id: u64, update: impl FnOnce(&mut Progress) -> T) -> T {
        let mut phase = self.lock();
        let admission = phase
            .admission_mut()
            .filter(|admission| admission.id == admission_id)
            .expect("the running admission stays present until it completes");
        update(&mut admission.progress)
    }
}

/// Phase one: admit, freeze, mint.
pub struct PrepareHarnessShutdown {
    transaction: Arc<HarnessShutdownTransaction>,
}

impl PrepareHarnessShutdown {
    pub fn new(transaction: Arc<HarnessShutdownTransaction>) -> Self {
        Self { transaction }
    }

    /// Admit the shutdown or join the one already admitted as a new holder.
    /// Never opens a second admission while one is live.
    pub fn execute(
        &self,
        request: PrepareShutdownRequest,
    ) -> Result<PreparedShutdown, HarnessShutdownError> {
        let transaction = &self.transaction;
        let mut phase = transaction.lock();
        if let Some(admission) = phase.admission_mut() {
            let token = admission.join(request.trigger);
            return Ok(PreparedShutdown {
                token,
                joined: true,
                reason: admission.reason,
            });
        }
        let frozen = transaction
            .lifecycle
            .lifecycle()
            .freeze()
            .map_err(|_| HarnessShutdownError::AlreadyTerminated)?;
        debug_assert_eq!(frozen, HarnessLifecycleState::Frozen);
        transaction.lifecycle.set_lifecycle(frozen);
        let mut admission = Admission {
            id: transaction.mint_admission_id(),
            reason: request.reason,
            triggers: Vec::new(),
            next_holder: 1,
            holders: BTreeSet::new(),
            released: BTreeSet::new(),
            progress: Progress::default(),
        };
        let token = admission.join(request.trigger);
        *phase = Phase::Prepared(admission);
        Ok(PreparedShutdown {
            token,
            joined: false,
            reason: request.reason,
        })
    }

    /// Give back one holder's admission (its ACK could not be delivered).
    /// Idempotent per holder. The freeze lifts only when the last holder
    /// leaves; once execution has begun there is nothing to give back.
    pub fn release(&self, token: &ShutdownToken) -> Result<ReleaseOutcome, HarnessShutdownError> {
        let transaction = &self.transaction;
        let mut phase = transaction.lock();
        let Some(admission) = phase.admission_mut() else {
            return Err(HarnessShutdownError::NotPrepared);
        };
        match admission.validate(token) {
            Ok(()) => {}
            Err(HarnessShutdownError::TokenReleased) => return Ok(ReleaseOutcome::AlreadyReleased),
            Err(error) => return Err(error),
        }
        admission.holders.remove(&token.holder());
        admission.released.insert(token.holder());
        match &*phase {
            Phase::Prepared(admission) if admission.holders.is_empty() => {
                let thawed = transaction
                    .lifecycle
                    .lifecycle()
                    .thaw()
                    .map_err(|e| HarnessShutdownError::LifecycleViolation(e.to_string()))?;
                transaction.lifecycle.set_lifecycle(thawed);
                *phase = Phase::Idle;
                Ok(ReleaseOutcome::Released)
            }
            Phase::Prepared(_) => Ok(ReleaseOutcome::StillHeld),
            Phase::Executing { .. } | Phase::Completed { .. } => {
                Ok(ReleaseOutcome::ExecutionUnderway)
            }
            Phase::Idle => unreachable!("an admission was present under the lock"),
        }
    }
}

/// Concrete collaborators of [`ExecuteHarnessShutdown`], named once.
pub struct ExecuteHarnessShutdownPorts {
    pub routing: Arc<dyn DirectChildRouting>,
    pub cancellation: Arc<dyn TurnCancellation>,
    pub persistence: Arc<dyn ShutdownSessionPersistence>,
    pub exit: Arc<dyn CompositionExitReadiness>,
    pub spawner: Arc<dyn ShutdownRunSpawner>,
}

/// Phase two: run the common teardown exactly once for the admitted token.
pub struct ExecuteHarnessShutdown {
    transaction: Arc<HarnessShutdownTransaction>,
    ports: Arc<ExecuteHarnessShutdownPorts>,
}

enum Admit {
    Run(u64),
    Join,
    Done(Outcome),
}

impl ExecuteHarnessShutdown {
    pub fn new(
        transaction: Arc<HarnessShutdownTransaction>,
        ports: ExecuteHarnessShutdownPorts,
    ) -> Self {
        Self {
            transaction,
            ports: Arc::new(ports),
        }
    }

    /// Start (or join) the teardown for `token` and wait for its outcome.
    /// The run itself is detached: dropping this future never stops it.
    pub async fn execute(&self, token: &ShutdownToken) -> Outcome {
        match self.admit(token)? {
            Admit::Run(admission_id) => {
                // The guard travels inside the run: a spawner that drops the
                // future unpolled still hands the admission back.
                let guard = RunGuard {
                    transaction: self.transaction.clone(),
                    armed: true,
                };
                let run = drive(guard, self.ports.clone(), admission_id);
                self.ports.spawner.spawn_shutdown_run(Box::pin(run));
                JoinOutcome::new(&self.transaction).await
            }
            Admit::Join => JoinOutcome::new(&self.transaction).await,
            Admit::Done(outcome) => outcome,
        }
    }

    fn admit(&self, token: &ShutdownToken) -> Result<Admit, HarnessShutdownError> {
        let mut phase = self.transaction.lock();
        let admission = phase.admission().ok_or(HarnessShutdownError::NotPrepared)?;
        admission.validate(token)?;
        match std::mem::replace(&mut *phase, Phase::Idle) {
            Phase::Prepared(admission) => {
                let id = admission.id;
                *phase = Phase::Executing {
                    admission,
                    wakers: Vec::new(),
                };
                Ok(Admit::Run(id))
            }
            executing @ Phase::Executing { .. } => {
                *phase = executing;
                Ok(Admit::Join)
            }
            Phase::Completed { admission, outcome } => {
                let result = outcome.clone();
                *phase = Phase::Completed { admission, outcome };
                Ok(Admit::Done(result))
            }
            Phase::Idle => unreachable!("an admission was present under the lock"),
        }
    }
}

/// The detached teardown. Each step records its effect before the next one
/// starts, and a run that is dropped midway leaves the phase `Prepared` with
/// that progress intact for the next `Execute`.
async fn drive(guard: RunGuard, ports: Arc<ExecuteHarnessShutdownPorts>, admission_id: u64) {
    let transaction = guard.transaction.clone();
    let (reason, done) = {
        let phase = transaction.lock();
        let admission = phase
            .admission()
            .expect("the running admission stays present until it completes");
        (admission.reason, admission.progress.clone())
    };
    if done.turn_cancelled.is_none() {
        let cancelled = ports.cancellation.cancel_in_flight_turn().await;
        transaction.with_progress(admission_id, |p| p.turn_cancelled = Some(cancelled));
    }
    if done.children.is_none() {
        let lineage = transaction.lifecycle.lineage();
        let mut shut_down = Vec::new();
        let mut failed = Vec::new();
        for child in lineage.direct_children() {
            match ports
                .routing
                .shutdown_child(child, ShutdownReason::ParentShutdown)
                .await
            {
                Ok(()) => shut_down.push(child.uuid.clone()),
                Err(error) => failed.push((child.uuid.clone(), error.to_string())),
            }
        }
        transaction.with_progress(admission_id, |p| p.children = Some((shut_down, failed)));
    }
    if done.persistence.is_none() {
        let persistence = match ports.persistence.persist_for_shutdown(reason).await {
            Ok(()) => PersistenceOutcome::Persisted,
            Err(detail) => PersistenceOutcome::Failed(detail),
        };
        transaction.with_progress(admission_id, |p| p.persistence = Some(persistence));
    }
    if !done.exit_signalled {
        ports.exit.signal_exit_ready(reason).await;
        transaction.with_progress(admission_id, |p| p.exit_signalled = true);
    }
    // Terminate last: everything the exit depends on has already happened,
    // and a run dropped before this point can still be resumed.
    let terminated = transaction
        .lifecycle
        .lifecycle()
        .terminate()
        .map_err(|e| HarnessShutdownError::LifecycleViolation(e.to_string()));
    let outcome = terminated.map(|state| {
        transaction.lifecycle.set_lifecycle(state);
        let phase = transaction.lock();
        let admission = phase
            .admission()
            .expect("the running admission stays present until it completes");
        let progress = admission.progress.clone();
        let (children_shut_down, children_failed) = progress.children.unwrap_or_default();
        ShutdownOutcome {
            reason,
            triggers: admission.triggers.clone(),
            turn_cancelled: progress.turn_cancelled.unwrap_or(false),
            children_shut_down,
            children_failed,
            persistence: progress
                .persistence
                .unwrap_or(PersistenceOutcome::Failed("not attempted".into())),
            exit_signalled: progress.exit_signalled,
        }
    });
    complete(&transaction, outcome);
    guard.disarm();
}

fn complete(transaction: &HarnessShutdownTransaction, outcome: Outcome) {
    let mut phase = transaction.lock();
    let Phase::Executing { admission, wakers } = std::mem::replace(&mut *phase, Phase::Idle) else {
        unreachable!("only the executing run completes the transaction");
    };
    *phase = Phase::Completed { admission, outcome };
    drop(phase);
    for waker in wakers {
        waker.wake();
    }
}

/// Restores the `Prepared` phase (progress included) if the detached run is
/// dropped before it completes, so joiners are released with
/// [`HarnessShutdownError::ExecutionInterrupted`] instead of hanging and the
/// next `Execute` resumes where this one stopped.
struct RunGuard {
    transaction: Arc<HarnessShutdownTransaction>,
    armed: bool,
}

impl RunGuard {
    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut phase = self.transaction.lock();
        let Phase::Executing { admission, wakers } = std::mem::replace(&mut *phase, Phase::Idle)
        else {
            unreachable!("only the executing run holds the guard");
        };
        *phase = Phase::Prepared(admission);
        drop(phase);
        for waker in wakers {
            waker.wake();
        }
    }
}

/// Resolves with the shared outcome once the detached run completes.
struct JoinOutcome<'a> {
    transaction: &'a HarnessShutdownTransaction,
}

impl<'a> JoinOutcome<'a> {
    fn new(transaction: &'a HarnessShutdownTransaction) -> Self {
        Self { transaction }
    }
}

impl Future for JoinOutcome<'_> {
    type Output = Outcome;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut phase = self.transaction.lock();
        match &mut *phase {
            Phase::Completed { outcome, .. } => Poll::Ready(outcome.clone()),
            Phase::Executing { wakers, .. } => {
                if !wakers.iter().any(|w| w.will_wake(cx.waker())) {
                    wakers.push(cx.waker().clone());
                }
                Poll::Pending
            }
            // The detached run was dropped and gave the admission back; the
            // joiner must call `Execute` again to resume it.
            Phase::Prepared(_) => Poll::Ready(Err(HarnessShutdownError::ExecutionInterrupted)),
            // Released by its last holder after an interruption and before
            // this joiner was polled: there is nothing to join.
            Phase::Idle => Poll::Ready(Err(HarnessShutdownError::NotPrepared)),
        }
    }
}

#[cfg(test)]
#[path = "harness_shutdown_tests.rs"]
mod tests;
