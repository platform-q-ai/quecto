//! Two-phase, idempotent harness shutdown (#1934).
//!
//! `Prepare` admits a shutdown and freezes new prompt/spawn work, handing back
//! an opaque token. The interface then writes and flushes the correlated ACK;
//! only after that does it hand the token to `Execute`. If the ACK cannot be
//! written the interface releases the admission instead, so a parent that
//! never received an ACK never sees the child vanish silently: it can tell a
//! graceful exit (ACK then EOF) from a loss (EOF with no ACK).
//!
//! Every trigger — protocol command, bound-parent connection closure, OS
//! signal — goes through the same transaction. The first one admits; later
//! ones join and observe the single outcome. No ACK is written here and no
//! wire type is named here.
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use crate::domain::subagent_teardown::{HarnessLifecycleState, ShutdownReason};

use super::super::dto::{
    HarnessShutdownError, PersistenceOutcome, PrepareShutdownRequest, PreparedShutdown,
    ReleaseOutcome, ShutdownOutcome, ShutdownToken, ShutdownTrigger,
};
use super::super::ports::{
    CompositionExitReadiness, DirectChildRouting, ShutdownClock, ShutdownSessionPersistence,
    SubagentLifecycleRepository, TurnCancellation,
};

type Outcome = Result<ShutdownOutcome, HarnessShutdownError>;

enum Phase {
    Idle,
    Prepared {
        token: ShutdownToken,
        reason: ShutdownReason,
        triggers: Vec<ShutdownTrigger>,
        /// Callers that hold the admission and have not released it.
        participants: usize,
    },
    Executing {
        token: ShutdownToken,
        wakers: Vec<Waker>,
    },
    Completed {
        token: ShutdownToken,
        outcome: Outcome,
    },
}

/// Process-wide mint serial so two transactions never hand out equal tokens.
static MINTED: AtomicU64 = AtomicU64::new(0);

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

    fn lock(&self) -> std::sync::MutexGuard<'_, Phase> {
        self.phase
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn mint_token(&self) -> ShutdownToken {
        // Opaque: a process-wide serial mixed with the clock so unrelated
        // admissions never mint equal tokens, even on a frozen clock.
        let serial = MINTED.fetch_add(1, Ordering::SeqCst).wrapping_add(1);
        let now = self.clock.now().0;
        ShutdownToken::mint(serial.rotate_left(32) ^ now)
    }

    /// Whether new prompt/spawn work may still be admitted.
    pub fn accepts_new_work(&self) -> bool {
        self.lifecycle.lifecycle().accepts_new_work()
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

    /// Admit the shutdown or join the one already admitted. Never mints a
    /// second token while an admission is live.
    pub fn execute(
        &self,
        request: PrepareShutdownRequest,
    ) -> Result<PreparedShutdown, HarnessShutdownError> {
        let transaction = &self.transaction;
        let mut phase = transaction.lock();
        match &mut *phase {
            Phase::Idle => {
                let frozen = transaction
                    .lifecycle
                    .lifecycle()
                    .freeze()
                    .map_err(|_| HarnessShutdownError::AlreadyTerminated)?;
                debug_assert_eq!(frozen, HarnessLifecycleState::Frozen);
                transaction.lifecycle.set_lifecycle(frozen);
                let token = transaction.mint_token();
                *phase = Phase::Prepared {
                    token: token.clone(),
                    reason: request.reason,
                    triggers: vec![request.trigger],
                    participants: 1,
                };
                Ok(PreparedShutdown {
                    token,
                    joined: false,
                    reason: request.reason,
                })
            }
            Phase::Prepared {
                token,
                reason,
                triggers,
                participants,
            } => {
                *participants += 1;
                triggers.push(request.trigger);
                Ok(PreparedShutdown {
                    token: token.clone(),
                    joined: true,
                    reason: *reason,
                })
            }
            // A trigger arriving mid-execution or after completion still
            // converges on the same outcome; its reason is already decided.
            Phase::Executing { token, .. } => Ok(PreparedShutdown {
                token: token.clone(),
                joined: true,
                reason: request.reason,
            }),
            Phase::Completed { token, outcome } => Ok(PreparedShutdown {
                token: token.clone(),
                joined: true,
                reason: outcome
                    .as_ref()
                    .map(|outcome| outcome.reason)
                    .unwrap_or(request.reason),
            }),
        }
    }

    /// Give back an admission whose ACK could not be delivered. The freeze
    /// lifts only when the last participant leaves; once execution has begun
    /// there is nothing to give back.
    pub fn release(&self, token: &ShutdownToken) -> Result<ReleaseOutcome, HarnessShutdownError> {
        let transaction = &self.transaction;
        let mut phase = transaction.lock();
        match &mut *phase {
            Phase::Idle => Err(HarnessShutdownError::NotPrepared),
            Phase::Prepared {
                token: admitted,
                participants,
                ..
            } => {
                if admitted != token {
                    return Err(HarnessShutdownError::UnknownToken);
                }
                assert!(*participants >= 1, "a prepared admission has a holder");
                *participants -= 1;
                if *participants > 0 {
                    return Ok(ReleaseOutcome::StillHeld);
                }
                let thawed = transaction
                    .lifecycle
                    .lifecycle()
                    .thaw()
                    .map_err(|e| HarnessShutdownError::LifecycleViolation(e.to_string()))?;
                transaction.lifecycle.set_lifecycle(thawed);
                *phase = Phase::Idle;
                Ok(ReleaseOutcome::Released)
            }
            Phase::Executing {
                token: admitted, ..
            }
            | Phase::Completed {
                token: admitted, ..
            } => {
                if admitted != token {
                    return Err(HarnessShutdownError::UnknownToken);
                }
                Ok(ReleaseOutcome::ExecutionUnderway)
            }
        }
    }
}

/// Phase two: run the common teardown exactly once for the admitted token.
pub struct ExecuteHarnessShutdown {
    transaction: Arc<HarnessShutdownTransaction>,
    routing: Arc<dyn DirectChildRouting>,
    cancellation: Arc<dyn TurnCancellation>,
    persistence: Arc<dyn ShutdownSessionPersistence>,
    exit: Arc<dyn CompositionExitReadiness>,
}

/// Concrete collaborators of [`ExecuteHarnessShutdown`], named once.
pub struct ExecuteHarnessShutdownPorts {
    pub routing: Arc<dyn DirectChildRouting>,
    pub cancellation: Arc<dyn TurnCancellation>,
    pub persistence: Arc<dyn ShutdownSessionPersistence>,
    pub exit: Arc<dyn CompositionExitReadiness>,
}

/// What the admitted run looked like when it started, so an interrupted
/// runner can hand the admission back intact.
struct Admitted {
    reason: ShutdownReason,
    triggers: Vec<ShutdownTrigger>,
    participants: usize,
}

enum Admission {
    Run(Admitted),
    Join,
    Done(Outcome),
}

/// Restores the `Prepared` phase if the running future is dropped before it
/// completes (a cancelled task), so joiners are released with
/// [`HarnessShutdownError::ExecutionInterrupted`] instead of hanging and the
/// next `Execute` for the same token runs the teardown again.
struct RunGuard<'a> {
    transaction: &'a HarnessShutdownTransaction,
    token: ShutdownToken,
    restore: Option<Admitted>,
}

impl RunGuard<'_> {
    fn disarm(mut self) {
        self.restore = None;
    }
}

impl Drop for RunGuard<'_> {
    fn drop(&mut self) {
        let Some(admitted) = self.restore.take() else {
            return;
        };
        let mut phase = self.transaction.lock();
        let Phase::Executing { wakers, .. } = std::mem::replace(
            &mut *phase,
            Phase::Prepared {
                token: self.token.clone(),
                reason: admitted.reason,
                triggers: admitted.triggers,
                participants: admitted.participants,
            },
        ) else {
            unreachable!("only the executing runner holds the guard");
        };
        drop(phase);
        for waker in wakers {
            waker.wake();
        }
    }
}

impl ExecuteHarnessShutdown {
    pub fn new(
        transaction: Arc<HarnessShutdownTransaction>,
        ports: ExecuteHarnessShutdownPorts,
    ) -> Self {
        Self {
            transaction,
            routing: ports.routing,
            cancellation: ports.cancellation,
            persistence: ports.persistence,
            exit: ports.exit,
        }
    }

    pub async fn execute(&self, token: &ShutdownToken) -> Outcome {
        match self.admit(token)? {
            Admission::Run(admitted) => {
                let guard = RunGuard {
                    transaction: &self.transaction,
                    token: token.clone(),
                    restore: Some(Admitted {
                        reason: admitted.reason,
                        triggers: admitted.triggers.clone(),
                        participants: admitted.participants,
                    }),
                };
                let outcome = self.run(admitted.reason, admitted.triggers).await;
                guard.disarm();
                self.complete(token, outcome.clone());
                outcome
            }
            Admission::Join => JoinOutcome::new(&self.transaction).await,
            Admission::Done(outcome) => outcome,
        }
    }

    fn admit(&self, token: &ShutdownToken) -> Result<Admission, HarnessShutdownError> {
        let mut phase = self.transaction.lock();
        match &*phase {
            Phase::Idle => Err(HarnessShutdownError::NotPrepared),
            Phase::Prepared {
                token: admitted, ..
            }
            | Phase::Executing {
                token: admitted, ..
            }
            | Phase::Completed {
                token: admitted, ..
            } if admitted != token => Err(HarnessShutdownError::UnknownToken),
            Phase::Prepared {
                reason,
                triggers,
                participants,
                ..
            } => {
                let admitted = Admitted {
                    reason: *reason,
                    triggers: triggers.clone(),
                    participants: *participants,
                };
                *phase = Phase::Executing {
                    token: token.clone(),
                    wakers: Vec::new(),
                };
                Ok(Admission::Run(admitted))
            }
            Phase::Executing { .. } => Ok(Admission::Join),
            Phase::Completed { outcome, .. } => Ok(Admission::Done(outcome.clone())),
        }
    }

    async fn run(&self, reason: ShutdownReason, triggers: Vec<ShutdownTrigger>) -> Outcome {
        let turn_cancelled = self.cancellation.cancel_in_flight_turn().await;
        let lineage = self.transaction.lifecycle.lineage();
        let mut children_shut_down = Vec::new();
        let mut children_failed = Vec::new();
        for child in lineage.direct_children() {
            match self
                .routing
                .shutdown_child(child, ShutdownReason::ParentShutdown)
                .await
            {
                Ok(()) => children_shut_down.push(child.uuid.clone()),
                Err(error) => children_failed.push((child.uuid.clone(), error.to_string())),
            }
        }
        let persistence = match self.persistence.persist_for_shutdown(reason).await {
            Ok(()) => PersistenceOutcome::Persisted,
            Err(detail) => PersistenceOutcome::Failed(detail),
        };
        let terminated = self
            .transaction
            .lifecycle
            .lifecycle()
            .terminate()
            .map_err(|e| HarnessShutdownError::LifecycleViolation(e.to_string()))?;
        self.transaction.lifecycle.set_lifecycle(terminated);
        self.exit.signal_exit_ready(reason).await;
        Ok(ShutdownOutcome {
            reason,
            triggers,
            turn_cancelled,
            children_shut_down,
            children_failed,
            persistence,
            exit_signalled: true,
        })
    }

    fn complete(&self, token: &ShutdownToken, outcome: Outcome) {
        let mut phase = self.transaction.lock();
        let wakers = match std::mem::replace(
            &mut *phase,
            Phase::Completed {
                token: token.clone(),
                outcome,
            },
        ) {
            Phase::Executing { wakers, .. } => wakers,
            _ => unreachable!("only the executing caller completes the transaction"),
        };
        drop(phase);
        for waker in wakers {
            waker.wake();
        }
    }
}

/// Resolves with the shared outcome once the executing caller completes.
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
            // The runner was dropped mid-teardown and gave the admission
            // back; the joiner must decide to run it itself.
            Phase::Prepared { .. } => Poll::Ready(Err(HarnessShutdownError::ExecutionInterrupted)),
            // Released by its last participant after an interruption and
            // before this joiner was polled: there is nothing to join.
            Phase::Idle => Poll::Ready(Err(HarnessShutdownError::NotPrepared)),
        }
    }
}

#[cfg(test)]
#[path = "harness_shutdown_tests.rs"]
mod tests;
