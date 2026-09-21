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
//!
//! The direct-children step is the fleet teardown
//! ([`TerminateAllDelegatedAgents`], #1938): every direct child is claimed,
//! asked over its edge, concluded through the owned-handle fallback and
//! compensated under a concurrency bound, so the harness exits only once its
//! subtree has settled. The fleet run is itself detached and joinable, so a
//! re-driven shutdown joins it rather than asking any child twice.
use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};

use crate::domain::ids::AgentUuid;
use crate::domain::subagent_teardown::{HarnessLifecycleState, ShutdownReason};

use super::super::dto::{
    FleetTeardownError, FleetTeardownOutcome, HarnessShutdownError, PersistenceOutcome,
    PrepareShutdownRequest, PreparedShutdown, ReleaseOutcome, ShutdownOutcome, ShutdownToken,
    ShutdownTrigger, TerminateAllDelegatedAgentsRequest,
};
use super::super::ports::{
    CompositionExitReadiness, ExitReadiness, OwnerExitAnnouncement, RetainedEnvironmentTeardown,
    ShutdownClock, ShutdownRunSpawner, ShutdownSessionPersistence, SubagentLifecycleRepository,
    TurnCancellation,
};
use super::terminate_all_delegated_agents::TerminateAllDelegatedAgents;

type Outcome = Result<ShutdownOutcome, HarnessShutdownError>;

/// Steps of the common teardown that have already produced their effect.
/// A re-driven run skips every recorded step.
#[derive(Debug, Clone, Default)]
struct Progress {
    /// A run was polled at least once: some port may already have been
    /// asked, even if its result was never recorded.
    started: bool,
    turn_cancelled: Option<bool>,
    children_shut_down: Vec<AgentUuid>,
    children_failed: Vec<(AgentUuid, String)>,
    /// The fleet teardown ran to completion (its per-child outcomes are
    /// recorded above). An interrupted fleet run leaves this unset so a
    /// re-drive of the admission runs it again; the claim ladder makes
    /// that safe.
    children_complete: bool,
    /// The owner had announced this exit when the fleet step ran (#2070):
    /// decided once, so a re-drive tears down on the same authority.
    owner_exit: Option<bool>,
    /// The emptied `retained` environments an owner exit ended; `None`
    /// until that step has run (or was skipped for an unannounced exit).
    retained_environments: Option<Vec<(String, Result<(), String>)>>,
    persistence: Option<PersistenceOutcome>,
    exit_signalled: bool,
}

impl Progress {
    /// Whether any effect has already happened for this admission.
    fn any_effect(&self) -> bool {
        self.started
            || self.turn_cancelled.is_some()
            || !self.children_shut_down.is_empty()
            || !self.children_failed.is_empty()
            || self.children_complete
            || self.retained_environments.is_some()
            || self.persistence.is_some()
            || self.exit_signalled
    }
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

    /// Number of callers currently parked on the in-flight run. Test probe
    /// so a test can make a joiner's registration observable before it
    /// interrupts the run.
    #[cfg(any(test, feature = "test-support"))]
    pub fn waiting_joiners(&self) -> usize {
        match &*self.lock() {
            Phase::Executing { wakers, .. } => wakers.len(),
            Phase::Idle | Phase::Prepared(_) | Phase::Completed { .. } => 0,
        }
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
    /// leaves *and nothing has run yet*; once any effect has happened (the
    /// run started, even if it was later interrupted) the admission is kept
    /// so a later trigger joins and resumes it instead of repeating effects.
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
            Phase::Prepared(admission) if admission.progress.any_effect() => {
                Ok(ReleaseOutcome::ExecutionUnderway)
            }
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
    /// The direct-children step: the fleet teardown (#1938).
    pub children: Arc<TerminateAllDelegatedAgents>,
    /// Whether the owner announced this exit (#2070).
    pub owner_exit: Arc<dyn OwnerExitAnnouncement>,
    /// The emptied `retained` environments an owner exit ends (#2070).
    pub retained: Arc<dyn RetainedEnvironmentTeardown>,
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

    /// Give up on driving the run for `token` after repeated interruption:
    /// composition is told to exit anyway, with the failure recorded, so an
    /// ACKed parent never sees silence. The admission stays as it is for any
    /// later trigger that can still resume it.
    pub async fn abandon(
        &self,
        token: &ShutdownToken,
        detail: impl Into<String>,
    ) -> Result<(), HarnessShutdownError> {
        let reason = {
            let phase = self.transaction.lock();
            let admission = phase.admission().ok_or(HarnessShutdownError::NotPrepared)?;
            admission.validate(token)?;
            admission.reason
        };
        self.ports
            .exit
            .signal_exit_ready(ExitReadiness::Abandoned {
                reason,
                detail: detail.into(),
            })
            .await;
        Ok(())
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
    transaction.with_progress(admission_id, |p| p.started = true);
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
    // Decided once, before the fleet is asked: an announcement that arrives
    // mid-teardown changes nothing already under way.
    let owner_exit = match done.owner_exit {
        Some(decided) => decided,
        None => {
            let announced = ports.owner_exit.announced();
            transaction.with_progress(admission_id, |p| p.owner_exit = Some(announced));
            announced
        }
    };
    if !done.children_complete {
        let fleet = settle_fleet(&ports, owner_exit).await;
        transaction.with_progress(admission_id, |p| match fleet {
            Ok(outcome) => {
                p.children_shut_down = outcome
                    .settled
                    .iter()
                    .map(|settled| settled.child.uuid.clone())
                    .collect();
                p.children_failed = outcome.unsettled;
                p.children_complete = true;
            }
            // The fleet run was dropped by its runtime: nothing is
            // recorded (the outcome reports no child settled) and the
            // shutdown still persists and signals exit, so an ACKed parent
            // never sees silence; a re-drive of an interrupted admission
            // runs the fleet again before finishing.
            Err(FleetTeardownError::Interrupted) => {}
        });
    }
    // The owner's exit also ends what this session left behind: boxes it
    // emptied and kept `retained` for a resume nobody will ask for now.
    // After the fleet, so a member finalised just now is not raced.
    if done.retained_environments.is_none() {
        let ended = if owner_exit {
            ports.retained.end_emptied_retained().await
        } else {
            Vec::new()
        };
        transaction.with_progress(admission_id, |p| p.retained_environments = Some(ended));
    }
    if done.persistence.is_none() {
        let persistence = match ports.persistence.persist_for_shutdown(reason).await {
            Ok(()) => PersistenceOutcome::Persisted,
            Err(detail) => PersistenceOutcome::Failed(detail),
        };
        transaction.with_progress(admission_id, |p| p.persistence = Some(persistence));
    }
    if !done.exit_signalled {
        ports
            .exit
            .signal_exit_ready(ExitReadiness::Completed(reason))
            .await;
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
        ShutdownOutcome {
            reason,
            triggers: admission.triggers.clone(),
            turn_cancelled: progress.turn_cancelled.unwrap_or(false),
            children_shut_down: progress.children_shut_down,
            children_failed: progress.children_failed,
            persistence: progress
                .persistence
                .unwrap_or(PersistenceOutcome::Failed("not attempted".into())),
            exit_signalled: progress.exit_signalled,
            owner_exit: progress.owner_exit.unwrap_or(false),
            retained_environments: progress.retained_environments.unwrap_or_default(),
        }
    });
    complete(&transaction, outcome);
    guard.disarm();
}

/// The fleet step. The fleet run is detached and joinable: a re-driven
/// shutdown joins the one in flight, and the claim ladder guarantees no
/// child is asked twice even when a fresh fleet run starts. A run this
/// shutdown merely **joined** was started by an operator (delete-all, a
/// session transition) while the harness was still accepting, and may have
/// read its lineage before a child that the freeze then closed off was
/// registered; so after a joined run the fleet is settled once more. The
/// lineage is closed post-freeze, so that pass is an empty read whenever
/// nothing slipped in, and its results are merged.
async fn settle_fleet(
    ports: &ExecuteHarnessShutdownPorts,
    owner_exit: bool,
) -> Result<FleetTeardownOutcome, FleetTeardownError> {
    use super::super::dto::FleetTeardownAuthority;
    // A shutdown may be a crash (#2070): it is the owner's word only when
    // the owner announced the exit beforehand.
    let authority = if owner_exit {
        FleetTeardownAuthority::Owner
    } else {
        FleetTeardownAuthority::Harness
    };
    let request = || TerminateAllDelegatedAgentsRequest {
        reason: ShutdownReason::ParentShutdown,
        authority,
    };
    let mut outcome = ports.children.execute(request()).await?;
    if outcome.joined {
        let sweep = ports.children.execute(request()).await?;
        for settled in sweep.settled {
            if !outcome.settled.contains(&settled) {
                outcome.settled.push(settled);
            }
        }
        for unsettled in sweep.unsettled {
            if !outcome.unsettled.contains(&unsettled) {
                outcome.unsettled.push(unsettled);
            }
        }
        // A child settled by the sweep is no longer unsettled.
        let settled = outcome.settled.clone();
        outcome
            .unsettled
            .retain(|(uuid, _)| !settled.iter().any(|s| &s.child.uuid == uuid));
        outcome.pruned.extend(sweep.pruned);
    }
    Ok(outcome)
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
#[path = "harness_shutdown_owner_exit_tests.rs"]
mod owner_exit_tests;
#[cfg(test)]
#[path = "harness_shutdown_sweep_tests.rs"]
mod sweep_tests;
#[cfg(test)]
#[path = "harness_shutdown_tests.rs"]
mod tests;
