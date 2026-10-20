//! #1909: the session's automatic turns, run through the swarm's control
//! generation: a provider failure suspends them, dated after the failure;
//! a resume since then re-arms them, a wake alone never does; an explicit
//! instruction re-arms any suspension.
use super::*;
use crate::application::swarm::ports::{PortFuture, lifecycle::CoordinatorBoardFuture};
use crate::domain::inference::services::turn_suspension::WakeOutcome;
use crate::domain::swarm::value_objects::lifecycle::RunStatus;
use crate::domain::swarm::value_objects::run_control::RunControlReceipt;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// A run at a control generation the test moves; `u64::MAX` fails a call.
struct Run {
    generation: AtomicU64,
    calls: AtomicUsize,
}

impl Run {
    fn at(generation: u64) -> Self {
        Self {
            generation: AtomicU64::new(generation),
            calls: AtomicUsize::new(0),
        }
    }
    fn moves_to(&self, generation: u64) {
        self.generation.store(generation, Ordering::SeqCst);
    }
}

impl SwarmRunControl for Run {
    fn apply(
        &self,
        action: RunControlAction,
    ) -> PortFuture<'_, Result<RunControlReceipt, DomainError>> {
        assert!(
            matches!(action, RunControlAction::Status),
            "only a status read"
        );
        self.calls.fetch_add(1, Ordering::SeqCst);
        let generation = self.generation.load(Ordering::SeqCst);
        Box::pin(async move {
            if generation == u64::MAX {
                return Err(DomainError::Tool("database is locked".into()));
            }
            Ok(RunControlReceipt {
                budget: None,
                wake_allowed: false,
                status: RunStatus::Running,
                outcome: None,
                reason: None,
                generation,
                wake_warnings: Vec::new(),
                resume_blockers: Vec::new(),
            })
        })
    }
    fn nudge_watch(&self) {}
    fn coordinator_board(&self) -> CoordinatorBoardFuture<'_> {
        Box::pin(async { Ok(None) })
    }
}

#[tokio::test]
async fn a_failure_is_dated_after_it_and_only_a_later_resume_re_arms() {
    let run = Run::at(4);
    let mut turns = ResumeAfterProviderFailure::new();
    turns.observe_control_generation(Some(3));
    turns.suspend_after_failure(&DomainError::Provider(
        "provider error (401): invalid credentials".into(),
    ));
    assert!(!turns.allowed());
    assert_eq!(turns.date_suspension(Some(&run)).await, Some(4));
    let dated = turns.turns.suspension().expect("suspended");
    assert_eq!(
        (dated.cause, dated.generation, dated.dated),
        (SuspensionCause::ProviderFailure, Some(4), true)
    );
    assert_eq!(turns.date_suspension(Some(&run)).await, None, "dated once");
    assert_eq!(
        turns.probe_while_suspended(&run).await.unwrap(),
        SuspendedProbe {
            outcome: WakeOutcome::StillSuspended,
            generation: 4
        },
        "a wake alone never clears a suspension"
    );
    run.moves_to(6);
    assert_eq!(
        turns.probe_while_suspended(&run).await.unwrap(),
        SuspendedProbe {
            outcome: WakeOutcome::RearmedByResume,
            generation: 6
        }
    );
    assert!(turns.allowed());
    assert_eq!(
        turns.take_owed_turn(),
        Some(OwedTurn::AfterResume),
        "the resume owes the run a turn"
    );
    assert_eq!(turns.take_owed_turn(), None, "once");
}

#[tokio::test]
async fn without_a_swarm_or_an_answer_the_pre_turn_generation_stands() {
    let mut turns = ResumeAfterProviderFailure::new();
    turns.observe_control_generation(Some(2));
    turns.suspend_after_failure(&DomainError::Provider(
        "provider error (401): invalid credentials".into(),
    ));
    assert_eq!(turns.date_suspension(None).await, None);
    let locked = Run::at(u64::MAX);
    assert_eq!(turns.date_suspension(Some(&locked)).await, None);
    let kept = turns.turns.suspension().expect("suspended");
    assert_eq!((kept.generation, kept.dated), (Some(2), false));
}

#[tokio::test]
async fn a_store_rejection_waits_for_an_instruction() {
    let run = Run::at(1);
    let mut turns = ResumeAfterProviderFailure::new();
    turns.suspend_after_store_rejection();
    assert_eq!(turns.date_suspension(Some(&run)).await, None, "never dated");
    assert_eq!(run.calls.load(Ordering::SeqCst), 0, "no store read for it");
    run.moves_to(9);
    assert_eq!(
        turns.probe_while_suspended(&run).await.unwrap(),
        SuspendedProbe {
            outcome: WakeOutcome::StillSuspended,
            generation: 9
        }
    );
    turns.start_turn(
        Work::Instruction(
            crate::domain::inference::services::turn_suspension::Instruction::FollowUp,
        ),
        None,
    );
    assert!(turns.allowed());
    assert_eq!(
        turns.take_owed_turn(),
        None,
        "an instruction owes no resume turn"
    );
}

#[tokio::test]
async fn a_probe_the_store_cannot_answer_is_the_stores_error() {
    let mut turns = ResumeAfterProviderFailure::new();
    turns.suspend(SuspensionCause::ProviderFailure, Some(1));
    let error = turns
        .probe_while_suspended(&Run::at(u64::MAX))
        .await
        .expect_err("the caller decides what a busy store means");
    assert!(error.to_string().contains("locked"), "{error}");
    assert!(!turns.allowed(), "still suspended");
}

/// #2618 review: a turn starts as its work: a prompt re-arms a suspended
/// session and dates a later failure at the generation given; automatic
/// work is admitted only while automatic turns run.
#[test]
fn a_turn_starts_as_its_work() {
    use crate::domain::inference::services::turn_suspension::Instruction;
    let mut turns = ResumeAfterProviderFailure::new();
    turns.suspend(SuspensionCause::ProviderFailure, Some(1));
    assert!(!turns.admits(Work::Automatic));
    assert!(turns.admits(Work::Instruction(Instruction::Prompt)));
    turns.start_turn(Work::Instruction(Instruction::Prompt), Some(5));
    assert!(turns.allowed());
    assert!(turns.admits(Work::Automatic));
    turns.suspend_after_failure(&DomainError::Provider(
        "provider error (401): invalid credentials".into(),
    ));
    assert_eq!(turns.turns.suspension().and_then(|s| s.generation), Some(5));
}

/// #2531: a recoverable failure (an Envoy 507 whose retries ran out) is
/// re-armed by the next wake, without a resume, and owes the run a turn to
/// continue the interrupted work; until then it asks to wake itself.
#[tokio::test]
async fn the_next_wake_re_arms_a_recoverable_failure() {
    let run = Run::at(4);
    let mut turns = ResumeAfterProviderFailure::new();
    turns.suspend_after_failure(&DomainError::Provider(
        "HTTP 507 from Codex: exceeded request buffer limit while retrying upstream".into(),
    ));
    assert_eq!(turns.date_suspension(Some(&run)).await, Some(4));
    assert!(turns.needs_self_wake(), "no one else may wake it");
    assert_eq!(
        turns.probe_while_suspended(&run).await.unwrap(),
        SuspendedProbe {
            outcome: WakeOutcome::RearmedAfterRecoverableFailure,
            generation: 4
        }
    );
    assert!(turns.allowed());
    assert!(!turns.needs_self_wake());
    assert_eq!(
        turns.take_owed_turn(),
        Some(OwedTurn::AfterRecoverableFailure),
        "the run is owed a turn to continue"
    );
    assert_eq!(turns.take_owed_turn(), None, "once");
}
