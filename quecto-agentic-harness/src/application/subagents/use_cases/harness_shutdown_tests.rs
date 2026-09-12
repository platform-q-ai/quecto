use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::*;
use crate::application::subagents::dto::{
    HarnessShutdownError, PersistenceOutcome, PrepareShutdownRequest, ReleaseOutcome,
    ShutdownTrigger,
};
use crate::application::subagents::ports::ExitReadiness;
use crate::domain::ids::AgentUuid;
use crate::domain::subagent_teardown::{HarnessLifecycleState, ShutdownReason};

use super::super::teardown_fakes::*;

struct Rig {
    lifecycle: Arc<FakeLifecycle>,
    routing: Arc<FakeRouting>,
    cancellation: Arc<FakeCancellation>,
    persistence: Arc<FakePersistence>,
    exit: Arc<FakeExit>,
    spawner: Arc<FakeSpawner>,
    prepare: PrepareHarnessShutdown,
    execute: Arc<ExecuteHarnessShutdown>,
}

fn rig_with(cancellation: Arc<FakeCancellation>) -> Rig {
    let lifecycle = FakeLifecycle::new(root_tree());
    let routing = FakeRouting::new();
    let persistence = FakePersistence::new();
    let exit = FakeExit::new();
    let spawner = FakeSpawner::new();
    let transaction = HarnessShutdownTransaction::new(lifecycle.clone(), FakeClock::at(1_000));
    let prepare = PrepareHarnessShutdown::new(transaction.clone());
    let execute = Arc::new(ExecuteHarnessShutdown::new(
        transaction,
        ExecuteHarnessShutdownPorts {
            routing: routing.clone(),
            cancellation: cancellation.clone(),
            persistence: persistence.clone(),
            exit: exit.clone(),
            spawner: spawner.clone(),
        },
    ));
    Rig {
        lifecycle,
        routing,
        cancellation,
        persistence,
        exit,
        spawner,
        prepare,
        execute,
    }
}

fn rig() -> Rig {
    rig_with(FakeCancellation::new(true))
}

fn foreign_token() -> ShutdownToken {
    let other = HarnessShutdownTransaction::new(FakeLifecycle::new(root_tree()), FakeClock::at(5));
    PrepareHarnessShutdown::new(other)
        .execute(protocol(ShutdownReason::OperatorRequest))
        .unwrap()
        .token
}

fn protocol(reason: ShutdownReason) -> PrepareShutdownRequest {
    PrepareShutdownRequest {
        reason,
        trigger: ShutdownTrigger::ProtocolCommand,
    }
}

fn signal() -> PrepareShutdownRequest {
    PrepareShutdownRequest {
        reason: ShutdownReason::TerminationSignal,
        trigger: ShutdownTrigger::TerminationSignal,
    }
}

fn nothing_ran(rig: &Rig) {
    assert!(rig.routing.calls().is_empty());
    assert_eq!(rig.cancellation.calls.load(Ordering::SeqCst), 0);
    assert!(rig.persistence.calls.lock().unwrap().is_empty());
    assert!(rig.exit.signalled.lock().unwrap().is_empty());
    assert_eq!(rig.spawner.spawned.load(Ordering::SeqCst), 0);
}

fn effects_once(rig: &Rig) {
    assert_eq!(rig.cancellation.calls.load(Ordering::SeqCst), 1);
    assert_eq!(rig.routing.calls().len(), 2);
    assert_eq!(rig.persistence.calls.lock().unwrap().len(), 1);
    assert_eq!(rig.exit.signalled.lock().unwrap().len(), 1);
}

#[test]
fn prepare_freezes_new_work_and_mints_an_opaque_token_without_running_anything() {
    let rig = rig();
    assert!(rig.lifecycle.lifecycle().accepts_new_work());
    let prepared = rig
        .prepare
        .execute(protocol(ShutdownReason::ParentShutdown))
        .unwrap();
    assert!(!prepared.joined);
    assert_eq!(prepared.reason, ShutdownReason::ParentShutdown);
    assert_eq!(format!("{:?}", prepared.token), "ShutdownToken(..)");
    assert_eq!(rig.lifecycle.lifecycle(), HarnessLifecycleState::Frozen);
    assert!(!rig.lifecycle.lifecycle().accepts_new_work());
    nothing_ran(&rig);
}

#[tokio::test]
async fn execute_detaches_the_run_and_records_the_common_teardown_in_order() {
    let rig = rig();
    let token = rig
        .prepare
        .execute(protocol(ShutdownReason::OperatorRequest))
        .unwrap()
        .token;
    let outcome = rig.execute.execute(&token).await.unwrap();
    assert_eq!(rig.spawner.spawned.load(Ordering::SeqCst), 1);
    assert!(outcome.turn_cancelled);
    assert_eq!(outcome.reason, ShutdownReason::OperatorRequest);
    assert_eq!(outcome.triggers, [ShutdownTrigger::ProtocolCommand]);
    // Only direct children are addressed; B and C belong to A's own shutdown.
    assert_eq!(
        outcome.children_shut_down,
        [AgentUuid::new("A"), AgentUuid::new("D")]
    );
    assert!(outcome.children_failed.is_empty());
    assert_eq!(
        rig.routing.calls(),
        [
            RoutingCall::Shutdown(identity("A", 1), ShutdownReason::ParentShutdown),
            RoutingCall::Shutdown(identity("D", 1), ShutdownReason::ParentShutdown),
        ]
    );
    assert_eq!(outcome.persistence, PersistenceOutcome::Persisted);
    assert_eq!(
        *rig.persistence.calls.lock().unwrap(),
        [ShutdownReason::OperatorRequest]
    );
    assert!(outcome.exit_signalled);
    assert_eq!(
        rig.exit.signalled(),
        [ExitReadiness::Completed(ShutdownReason::OperatorRequest)]
    );
    // Terminated is set last, after exit readiness was signalled.
    assert_eq!(
        *rig.lifecycle.transitions.lock().unwrap(),
        [
            HarnessLifecycleState::Frozen,
            HarnessLifecycleState::Terminated
        ]
    );
}

#[tokio::test]
async fn execute_without_prepare_or_with_a_foreign_token_has_no_effect() {
    let rig = rig();
    assert_eq!(
        rig.execute.execute(&foreign_token()).await,
        Err(HarnessShutdownError::NotPrepared)
    );
    let right = rig
        .prepare
        .execute(protocol(ShutdownReason::OperatorRequest))
        .unwrap()
        .token;
    let wrong = foreign_token();
    assert_ne!(wrong, right);
    assert_eq!(
        rig.execute.execute(&wrong).await,
        Err(HarnessShutdownError::UnknownToken)
    );
    nothing_ran(&rig);
    assert!(rig.execute.execute(&right).await.is_ok());
    // Even after completion a foreign token cannot join the outcome.
    assert_eq!(
        rig.execute.execute(&wrong).await,
        Err(HarnessShutdownError::UnknownToken)
    );
}

#[test]
fn every_participant_gets_its_own_token_on_one_admission() {
    let rig = rig();
    let first = rig
        .prepare
        .execute(protocol(ShutdownReason::ParentShutdown))
        .unwrap();
    let second = rig.prepare.execute(signal()).unwrap();
    assert!(!first.joined);
    assert!(second.joined);
    // Distinct holder tokens, one admission: the admitting reason wins.
    assert_ne!(first.token, second.token);
    assert_eq!(second.reason, ShutdownReason::ParentShutdown);
    assert_eq!(rig.lifecycle.transitions.lock().unwrap().len(), 1);
}

#[test]
fn release_is_idempotent_per_holder_and_thaws_only_when_every_holder_released() {
    let rig = rig();
    let a = rig
        .prepare
        .execute(protocol(ShutdownReason::ParentShutdown))
        .unwrap()
        .token;
    let b = rig.prepare.execute(signal()).unwrap().token;
    // A releases; B still holds. A's second release is a distinct no-op and
    // can never be mistaken for B's.
    assert_eq!(rig.prepare.release(&a), Ok(ReleaseOutcome::StillHeld));
    assert_eq!(rig.prepare.release(&a), Ok(ReleaseOutcome::AlreadyReleased));
    assert_eq!(rig.prepare.release(&a), Ok(ReleaseOutcome::AlreadyReleased));
    assert_eq!(rig.lifecycle.lifecycle(), HarnessLifecycleState::Frozen);
    assert_eq!(rig.prepare.release(&b), Ok(ReleaseOutcome::Released));
    assert_eq!(rig.lifecycle.lifecycle(), HarnessLifecycleState::Accepting);
    assert_eq!(
        rig.prepare.release(&b),
        Err(HarnessShutdownError::NotPrepared)
    );
    nothing_ran(&rig);
}

#[tokio::test]
async fn a_released_holder_can_neither_execute_nor_block_others() {
    let rig = rig();
    let a = rig
        .prepare
        .execute(protocol(ShutdownReason::ParentShutdown))
        .unwrap()
        .token;
    let b = rig.prepare.execute(signal()).unwrap().token;
    assert_eq!(rig.prepare.release(&a), Ok(ReleaseOutcome::StillHeld));
    assert_eq!(
        rig.execute.execute(&a).await,
        Err(HarnessShutdownError::TokenReleased)
    );
    nothing_ran(&rig);
    assert!(rig.execute.execute(&b).await.is_ok());
    effects_once(&rig);
    // A spent token stays spent after execution too.
    assert_eq!(
        rig.execute.execute(&a).await,
        Err(HarnessShutdownError::TokenReleased)
    );
    assert_eq!(rig.prepare.release(&a), Ok(ReleaseOutcome::AlreadyReleased));
    assert_eq!(
        rig.prepare.release(&b),
        Ok(ReleaseOutcome::ExecutionUnderway)
    );
}

#[tokio::test]
async fn a_released_admission_mints_a_fresh_one_and_the_stale_token_is_dead() {
    let rig = rig();
    let stale = rig
        .prepare
        .execute(protocol(ShutdownReason::ParentShutdown))
        .unwrap()
        .token;
    assert_eq!(rig.prepare.release(&stale), Ok(ReleaseOutcome::Released));
    assert_eq!(
        rig.execute.execute(&stale).await,
        Err(HarnessShutdownError::NotPrepared)
    );
    let fresh = rig
        .prepare
        .execute(protocol(ShutdownReason::ParentShutdown))
        .unwrap();
    assert!(!fresh.joined);
    assert_ne!(fresh.token, stale);
    assert_eq!(
        rig.execute.execute(&stale).await,
        Err(HarnessShutdownError::UnknownToken)
    );
    assert_eq!(
        rig.prepare.release(&stale),
        Err(HarnessShutdownError::UnknownToken)
    );
    nothing_ran(&rig);
    assert!(rig.execute.execute(&fresh.token).await.is_ok());
}

#[tokio::test]
async fn duplicate_execute_joins_the_detached_run_and_effects_happen_once() {
    let rig = rig_with(FakeCancellation::holding());
    let a = rig
        .prepare
        .execute(protocol(ShutdownReason::ParentShutdown))
        .unwrap()
        .token;
    let first = tokio::spawn({
        let execute = rig.execute.clone();
        let token = a.clone();
        async move { execute.execute(&token).await }
    });
    while rig.cancellation.calls.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    // A late trigger joins as its own holder and can execute with it.
    let late = rig.prepare.execute(signal()).unwrap();
    assert!(late.joined);
    assert_ne!(late.token, a);
    let second = tokio::spawn({
        let execute = rig.execute.clone();
        let token = late.token.clone();
        async move { execute.execute(&token).await }
    });
    tokio::task::yield_now().await;
    assert!(!second.is_finished(), "joiner must wait for the single run");
    assert_eq!(
        rig.prepare.release(&late.token),
        Ok(ReleaseOutcome::ExecutionUnderway)
    );
    rig.cancellation.gate.notify_one();
    let first = first.await.unwrap().unwrap();
    let second = second.await.unwrap().unwrap();
    assert_eq!(first, second);
    // Joiners during execution are counted as triggers of the outcome.
    assert_eq!(
        first.triggers,
        [
            ShutdownTrigger::ProtocolCommand,
            ShutdownTrigger::TerminationSignal
        ]
    );
    let third = rig.execute.execute(&a).await.unwrap();
    assert_eq!(third, first);
    effects_once(&rig);
    assert_eq!(rig.spawner.spawned.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn dropping_the_caller_never_stops_the_detached_run() {
    let rig = rig_with(FakeCancellation::holding());
    let token = rig
        .prepare
        .execute(protocol(ShutdownReason::ParentShutdown))
        .unwrap()
        .token;
    let caller = tokio::spawn({
        let execute = rig.execute.clone();
        let token = token.clone();
        async move { execute.execute(&token).await }
    });
    while rig.cancellation.calls.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    rig.cancellation.gate.notify_one();
    rig.spawner.latest_finished().await;
    effects_once(&rig);
    assert_eq!(rig.lifecycle.lifecycle(), HarnessLifecycleState::Terminated);
    let outcome = rig.execute.execute(&token).await.unwrap();
    assert!(outcome.exit_signalled);
}

#[tokio::test]
async fn simultaneous_os_parent_and_protocol_triggers_converge_on_one_outcome() {
    let rig = rig();
    let requests = [
        signal(),
        PrepareShutdownRequest {
            reason: ShutdownReason::ParentConnectionLost,
            trigger: ShutdownTrigger::ParentConnectionClosed,
        },
        protocol(ShutdownReason::ParentShutdown),
    ];
    let tokens: Vec<_> = requests
        .iter()
        .cloned()
        .map(|request| rig.prepare.execute(request).unwrap().token)
        .collect();
    assert_ne!(tokens[0], tokens[1]);
    assert_ne!(tokens[1], tokens[2]);
    let mut outcomes = Vec::new();
    for token in &tokens {
        outcomes.push(rig.execute.execute(token).await.unwrap());
    }
    assert!(outcomes.iter().all(|outcome| outcome == &outcomes[0]));
    assert_eq!(outcomes[0].reason, ShutdownReason::TerminationSignal);
    assert_eq!(
        outcomes[0].triggers,
        [
            ShutdownTrigger::TerminationSignal,
            ShutdownTrigger::ParentConnectionClosed,
            ShutdownTrigger::ProtocolCommand,
        ]
    );
    effects_once(&rig);
    // After completion, a fresh prepare still joins: the harness is gone.
    let post = rig
        .prepare
        .execute(protocol(ShutdownReason::OperatorRequest))
        .unwrap();
    assert!(post.joined);
    assert_eq!(post.reason, ShutdownReason::TerminationSignal);
    assert_eq!(rig.execute.execute(&post.token).await.unwrap(), outcomes[0]);
    assert_eq!(rig.lifecycle.lifecycle(), HarnessLifecycleState::Terminated);
}

#[tokio::test]
async fn child_and_persistence_failures_are_reported_not_fatal() {
    let rig = rig();
    rig.routing
        .unreachable
        .lock()
        .unwrap()
        .push(AgentUuid::new("A"));
    *rig.persistence.fail_with.lock().unwrap() = Some("disk full".into());
    let token = rig
        .prepare
        .execute(protocol(ShutdownReason::ParentShutdown))
        .unwrap()
        .token;
    let outcome = rig.execute.execute(&token).await.unwrap();
    assert_eq!(outcome.children_shut_down, [AgentUuid::new("D")]);
    assert_eq!(
        outcome.children_failed,
        [(AgentUuid::new("A"), "unreachable: socket closed".to_owned())]
    );
    assert_eq!(
        outcome.persistence,
        PersistenceOutcome::Failed("disk full".into())
    );
    assert!(outcome.exit_signalled);
    assert_eq!(rig.lifecycle.lifecycle(), HarnessLifecycleState::Terminated);
}

#[test]
fn prepare_on_a_terminated_harness_is_rejected() {
    let rig = rig();
    rig.lifecycle.force(HarnessLifecycleState::Terminated);
    assert_eq!(
        rig.prepare
            .execute(protocol(ShutdownReason::ParentShutdown)),
        Err(HarnessShutdownError::AlreadyTerminated)
    );
    assert!(rig.lifecycle.transitions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_repository_that_lost_the_freeze_is_a_lifecycle_violation_shared_by_joiners() {
    let rig = rig();
    let token = rig
        .prepare
        .execute(protocol(ShutdownReason::ParentShutdown))
        .unwrap()
        .token;
    rig.lifecycle.force(HarnessLifecycleState::Accepting);
    let error = rig.execute.execute(&token).await.unwrap_err();
    assert_eq!(
        error,
        HarnessShutdownError::LifecycleViolation("cannot terminate from Accepting".into())
    );
    assert_eq!(rig.execute.execute(&token).await, Err(error));
    // Exit readiness was signalled before the terminate step failed.
    assert_eq!(rig.exit.signalled().len(), 1);
}

#[test]
fn terminal_error_vocabulary_is_stable() {
    let rendered = [
        HarnessShutdownError::NotPrepared.to_string(),
        HarnessShutdownError::UnknownToken.to_string(),
        HarnessShutdownError::TokenReleased.to_string(),
        HarnessShutdownError::AlreadyTerminated.to_string(),
        HarnessShutdownError::LifecycleViolation("x".into()).to_string(),
        HarnessShutdownError::ExecutionInterrupted.to_string(),
    ];
    assert_eq!(
        rendered,
        [
            "no shutdown has been prepared",
            "shutdown token is not the admitted one",
            "shutdown token was already released",
            "harness already terminated",
            "lifecycle violation: x",
            "shutdown execution was interrupted",
        ]
    );
}

#[tokio::test]
async fn re_driving_the_same_admission_skips_every_completed_step() {
    let rig = rig();
    rig.exit.hold.store(true, Ordering::SeqCst);
    let token = rig
        .prepare
        .execute(protocol(ShutdownReason::ParentShutdown))
        .unwrap()
        .token;
    let joiner = tokio::spawn({
        let execute = rig.execute.clone();
        let token = token.clone();
        async move { execute.execute(&token).await }
    });
    while rig.exit.attempts.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    rig.spawner.abort_latest().await;
    assert_eq!(
        joiner.await.unwrap(),
        Err(HarnessShutdownError::ExecutionInterrupted)
    );
    // Now exit readiness resolves; the re-run must only do that step.
    rig.exit.hold.store(false, Ordering::SeqCst);
    let outcome = rig.execute.execute(&token).await.unwrap();
    assert!(outcome.turn_cancelled);
    assert_eq!(outcome.children_shut_down.len(), 2);
    assert_eq!(outcome.persistence, PersistenceOutcome::Persisted);
    assert!(outcome.exit_signalled);
    effects_once(&rig);
    assert_eq!(rig.exit.attempts.load(Ordering::SeqCst), 2);
    assert_eq!(rig.spawner.spawned.load(Ordering::SeqCst), 2);
    assert_eq!(rig.lifecycle.lifecycle(), HarnessLifecycleState::Terminated);
}

#[tokio::test]
async fn release_after_an_interrupted_run_keeps_the_admission_and_a_new_prepare_resumes_it() {
    let rig = rig();
    rig.exit.hold.store(true, Ordering::SeqCst);
    let token = rig
        .prepare
        .execute(protocol(ShutdownReason::ParentShutdown))
        .unwrap()
        .token;
    let joiner = tokio::spawn({
        let execute = rig.execute.clone();
        let token = token.clone();
        async move { execute.execute(&token).await }
    });
    while rig.exit.attempts.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    rig.spawner.abort_latest().await;
    assert_eq!(
        joiner.await.unwrap(),
        Err(HarnessShutdownError::ExecutionInterrupted)
    );
    // Effects have run: the only holder releasing must NOT thaw or discard
    // the progress, otherwise a new admission would repeat every effect.
    assert_eq!(
        rig.prepare.release(&token),
        Ok(ReleaseOutcome::ExecutionUnderway)
    );
    assert_eq!(rig.lifecycle.lifecycle(), HarnessLifecycleState::Frozen);
    let later = rig.prepare.execute(signal()).unwrap();
    assert!(later.joined, "a later trigger joins the same admission");
    rig.exit.hold.store(false, Ordering::SeqCst);
    let outcome = rig.execute.execute(&later.token).await.unwrap();
    assert_eq!(
        outcome.triggers,
        [
            ShutdownTrigger::ProtocolCommand,
            ShutdownTrigger::TerminationSignal
        ]
    );
    effects_once(&rig);
    assert_eq!(rig.lifecycle.lifecycle(), HarnessLifecycleState::Terminated);
}

#[tokio::test]
async fn a_run_interrupted_mid_children_never_re_sends_to_a_recorded_child() {
    let rig = rig();
    *rig.routing.hold_child.lock().unwrap() = Some(AgentUuid::new("D"));
    let token = rig
        .prepare
        .execute(protocol(ShutdownReason::ParentShutdown))
        .unwrap()
        .token;
    let joiner = tokio::spawn({
        let execute = rig.execute.clone();
        let token = token.clone();
        async move { execute.execute(&token).await }
    });
    // A was shut down and recorded; the run is parked on D.
    while rig.routing.calls().len() < 2 {
        tokio::task::yield_now().await;
    }
    rig.spawner.abort_latest().await;
    assert_eq!(
        joiner.await.unwrap(),
        Err(HarnessShutdownError::ExecutionInterrupted)
    );
    *rig.routing.hold_child.lock().unwrap() = None;
    let outcome = rig.execute.execute(&token).await.unwrap();
    assert_eq!(
        outcome.children_shut_down,
        [AgentUuid::new("A"), AgentUuid::new("D")]
    );
    // A once, D twice (its first attempt was interrupted before its record);
    // the port contract makes that second D request a join, not a repeat.
    let calls = rig.routing.calls();
    assert_eq!(calls.len(), 3);
    assert_eq!(
        calls
            .iter()
            .filter(|c| matches!(c, RoutingCall::Shutdown(id, _) if id.uuid.as_str() == "A"))
            .count(),
        1
    );
    assert_eq!(rig.cancellation.calls.load(Ordering::SeqCst), 1);
    assert_eq!(rig.persistence.calls.lock().unwrap().len(), 1);
    assert_eq!(rig.exit.signalled().len(), 1);
}

#[tokio::test]
async fn abandon_signals_exit_readiness_with_the_failure_and_keeps_the_admission() {
    let rig = rig();
    let token = rig
        .prepare
        .execute(protocol(ShutdownReason::ParentShutdown))
        .unwrap()
        .token;
    rig.execute.abandon(&token, "runtime gone").await.unwrap();
    assert_eq!(
        rig.exit.signalled(),
        [ExitReadiness::Abandoned {
            reason: ShutdownReason::ParentShutdown,
            detail: "runtime gone".into(),
        }]
    );
    assert_eq!(rig.lifecycle.lifecycle(), HarnessLifecycleState::Frozen);
    assert_eq!(
        rig.execute.abandon(&foreign_token(), "x").await,
        Err(HarnessShutdownError::UnknownToken)
    );
    assert_eq!(
        ExitReadiness::Abandoned {
            reason: ShutdownReason::ParentShutdown,
            detail: String::new()
        }
        .reason(),
        ShutdownReason::ParentShutdown
    );
}

#[tokio::test]
async fn a_spawner_that_drops_the_run_is_reported_as_interrupted_not_hung() {
    let rig = rig();
    rig.spawner.drop_next.store(1, Ordering::SeqCst);
    let token = rig
        .prepare
        .execute(protocol(ShutdownReason::ParentShutdown))
        .unwrap()
        .token;
    assert_eq!(
        rig.execute.execute(&token).await,
        Err(HarnessShutdownError::ExecutionInterrupted)
    );
    assert_eq!(rig.lifecycle.lifecycle(), HarnessLifecycleState::Frozen);
    assert_eq!(rig.cancellation.calls.load(Ordering::SeqCst), 0);
    assert!(rig.execute.execute(&token).await.is_ok());
    effects_once(&rig);
}

#[test]
fn tokens_are_unique_per_admission_and_clock_mixed() {
    let lifecycle = FakeLifecycle::new(root_tree());
    let clock = FakeClock::at(7);
    let transaction = HarnessShutdownTransaction::new(lifecycle.clone(), clock.clone());
    let prepare = PrepareHarnessShutdown::new(transaction.clone());
    let first = prepare
        .execute(protocol(ShutdownReason::ParentShutdown))
        .unwrap()
        .token;
    prepare.release(&first).unwrap();
    clock.0.store(8, Ordering::SeqCst);
    let second = prepare
        .execute(protocol(ShutdownReason::ParentShutdown))
        .unwrap()
        .token;
    assert_ne!(first, second);
    assert!(!transaction.accepts_new_work());
}
