//! #1909: the session's automatic turns, run through the swarm's control
//! generation: a provider failure suspends them, dated after the failure;
//! a resume since then re-arms them, a wake alone never does; an explicit
//! instruction re-arms any suspension.
use super::*;
use crate::application::swarm::ports::{CoordinatorBoardFuture, PortFuture};
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
    turns.suspend_after_failure();
    assert!(!turns.allowed());
    assert_eq!(turns.date_suspension(Some(&run)).await, Some(4));
    let dated = turns.suspension().expect("suspended");
    assert_eq!(
        (dated.cause, dated.generation, dated.dated),
        (SuspensionCause::ProviderFailure, Some(4), true)
    );
    assert_eq!(turns.date_suspension(Some(&run)).await, None, "dated once");
    assert_eq!(
        turns.probe_while_suspended(&run).await.unwrap(),
        SuspendedProbe::StillSuspended { generation: 4 },
        "a wake alone never clears a suspension"
    );
    run.moves_to(6);
    assert_eq!(
        turns.probe_while_suspended(&run).await.unwrap(),
        SuspendedProbe::Rearmed { generation: 6 }
    );
    assert!(turns.allowed());
    assert!(turns.take_owed_turn(), "the resume owes the run a turn");
    assert!(!turns.take_owed_turn(), "once");
}

#[tokio::test]
async fn without_a_swarm_or_an_answer_the_pre_turn_generation_stands() {
    let mut turns = ResumeAfterProviderFailure::new();
    turns.observe_control_generation(Some(2));
    turns.suspend_after_failure();
    assert_eq!(turns.date_suspension(None).await, None);
    let locked = Run::at(u64::MAX);
    assert_eq!(turns.date_suspension(Some(&locked)).await, None);
    let kept = turns.suspension().expect("suspended");
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
        SuspendedProbe::StillSuspended { generation: 9 }
    );
    turns.rearm(Instruction::FollowUp);
    assert!(turns.allowed());
    assert!(
        !turns.take_owed_turn(),
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
