use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::*;
use crate::application::subagents::dto::{
    HarnessShutdownError, PersistenceOutcome, PrepareShutdownRequest, ReleaseOutcome,
    ShutdownTrigger,
};
use crate::domain::ids::AgentUuid;
use crate::domain::subagent_teardown::{HarnessLifecycleState, ShutdownReason};

use super::super::teardown_fakes::*;

struct Rig {
    lifecycle: Arc<FakeLifecycle>,
    routing: Arc<FakeRouting>,
    cancellation: Arc<FakeCancellation>,
    persistence: Arc<FakePersistence>,
    exit: Arc<FakeExit>,
    prepare: PrepareHarnessShutdown,
    execute: Arc<ExecuteHarnessShutdown>,
}

fn rig_with(cancellation: Arc<FakeCancellation>) -> Rig {
    let lifecycle = FakeLifecycle::new(root_tree());
    let routing = FakeRouting::new();
    let persistence = FakePersistence::new();
    let exit = FakeExit::new();
    let transaction = HarnessShutdownTransaction::new(lifecycle.clone(), FakeClock::at(1_000));
    let prepare = PrepareHarnessShutdown::new(transaction.clone());
    let execute = Arc::new(ExecuteHarnessShutdown::new(
        transaction,
        ExecuteHarnessShutdownPorts {
            routing: routing.clone(),
            cancellation: cancellation.clone(),
            persistence: persistence.clone(),
            exit: exit.clone(),
        },
    ));
    Rig {
        lifecycle,
        routing,
        cancellation,
        persistence,
        exit,
        prepare,
        execute,
    }
}

fn rig() -> Rig {
    rig_with(FakeCancellation::new(true))
}

fn protocol(reason: ShutdownReason) -> PrepareShutdownRequest {
    PrepareShutdownRequest {
        reason,
        trigger: ShutdownTrigger::ProtocolCommand,
    }
}

fn nothing_ran(rig: &Rig) {
    assert!(rig.routing.calls().is_empty());
    assert_eq!(rig.cancellation.calls.load(Ordering::SeqCst), 0);
    assert!(rig.persistence.calls.lock().unwrap().is_empty());
    assert!(rig.exit.signalled.lock().unwrap().is_empty());
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
async fn execute_runs_the_common_teardown_in_order_for_the_admitted_token() {
    let rig = rig();
    let token = rig
        .prepare
        .execute(protocol(ShutdownReason::OperatorRequest))
        .unwrap()
        .token;
    let outcome = rig.execute.execute(&token).await.unwrap();
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
        *rig.exit.signalled.lock().unwrap(),
        [ShutdownReason::OperatorRequest]
    );
    assert_eq!(
        *rig.lifecycle.transitions.lock().unwrap(),
        [
            HarnessLifecycleState::Frozen,
            HarnessLifecycleState::Terminated
        ]
    );
}

#[tokio::test]
async fn execute_without_prepare_is_a_terminal_error_with_no_effect() {
    let rig = rig();
    let foreign =
        HarnessShutdownTransaction::new(FakeLifecycle::new(root_tree()), FakeClock::at(5));
    let foreign_token = PrepareHarnessShutdown::new(foreign)
        .execute(protocol(ShutdownReason::OperatorRequest))
        .unwrap()
        .token;
    assert_eq!(rig.lifecycle.lifecycle(), HarnessLifecycleState::Accepting);
    assert_eq!(
        rig.execute.execute(&foreign_token).await,
        Err(HarnessShutdownError::NotPrepared)
    );
    nothing_ran(&rig);
}

#[tokio::test]
async fn execute_with_the_wrong_token_is_rejected_and_the_admission_survives() {
    let rig = rig();
    let other = HarnessShutdownTransaction::new(FakeLifecycle::new(root_tree()), FakeClock::at(5));
    let wrong = PrepareHarnessShutdown::new(other)
        .execute(protocol(ShutdownReason::OperatorRequest))
        .unwrap()
        .token;
    let right = rig
        .prepare
        .execute(protocol(ShutdownReason::OperatorRequest))
        .unwrap()
        .token;
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
fn duplicate_prepare_joins_the_live_admission_instead_of_minting_a_second_token() {
    let rig = rig();
    let first = rig
        .prepare
        .execute(protocol(ShutdownReason::ParentShutdown))
        .unwrap();
    let second = rig
        .prepare
        .execute(PrepareShutdownRequest {
            reason: ShutdownReason::TerminationSignal,
            trigger: ShutdownTrigger::TerminationSignal,
        })
        .unwrap();
    assert!(second.joined);
    assert_eq!(second.token, first.token);
    // The admitting trigger's reason wins; joiners inherit it.
    assert_eq!(second.reason, ShutdownReason::ParentShutdown);
    assert_eq!(rig.lifecycle.transitions.lock().unwrap().len(), 1);
}

#[test]
fn release_lifts_the_freeze_only_when_the_last_participant_leaves() {
    let rig = rig();
    let token = rig
        .prepare
        .execute(protocol(ShutdownReason::ParentShutdown))
        .unwrap()
        .token;
    rig.prepare
        .execute(PrepareShutdownRequest {
            reason: ShutdownReason::ParentConnectionLost,
            trigger: ShutdownTrigger::ParentConnectionClosed,
        })
        .unwrap();
    assert_eq!(rig.prepare.release(&token), Ok(ReleaseOutcome::StillHeld));
    assert_eq!(rig.lifecycle.lifecycle(), HarnessLifecycleState::Frozen);
    assert_eq!(rig.prepare.release(&token), Ok(ReleaseOutcome::Released));
    assert_eq!(rig.lifecycle.lifecycle(), HarnessLifecycleState::Accepting);
    assert_eq!(
        rig.prepare.release(&token),
        Err(HarnessShutdownError::NotPrepared)
    );
    nothing_ran(&rig);
}

#[tokio::test]
async fn a_released_token_can_never_execute_and_a_fresh_prepare_mints_a_new_one() {
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
    nothing_ran(&rig);
    assert!(rig.execute.execute(&fresh.token).await.is_ok());
}

#[test]
fn release_with_a_foreign_token_is_rejected_and_keeps_the_admission() {
    let rig = rig();
    let token = rig
        .prepare
        .execute(protocol(ShutdownReason::ParentShutdown))
        .unwrap()
        .token;
    let other = HarnessShutdownTransaction::new(FakeLifecycle::new(root_tree()), FakeClock::at(5));
    let foreign = PrepareHarnessShutdown::new(other)
        .execute(protocol(ShutdownReason::OperatorRequest))
        .unwrap()
        .token;
    assert_eq!(
        rig.prepare.release(&foreign),
        Err(HarnessShutdownError::UnknownToken)
    );
    assert_eq!(rig.lifecycle.lifecycle(), HarnessLifecycleState::Frozen);
    assert_eq!(rig.prepare.release(&token), Ok(ReleaseOutcome::Released));
}

#[tokio::test]
async fn duplicate_execute_joins_the_in_flight_run_and_effects_happen_once() {
    let rig = rig_with(FakeCancellation::holding());
    let token = rig
        .prepare
        .execute(protocol(ShutdownReason::ParentShutdown))
        .unwrap()
        .token;
    let first = tokio::spawn({
        let execute = rig.execute.clone();
        let token = token.clone();
        async move { execute.execute(&token).await }
    });
    // Wait until the first caller is parked inside cancellation.
    while rig.cancellation.calls.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    let second = tokio::spawn({
        let execute = rig.execute.clone();
        let token = token.clone();
        async move { execute.execute(&token).await }
    });
    tokio::task::yield_now().await;
    assert!(!second.is_finished(), "joiner must wait for the single run");
    // A late trigger arriving mid-execution converges too.
    let late = rig
        .prepare
        .execute(PrepareShutdownRequest {
            reason: ShutdownReason::TerminationSignal,
            trigger: ShutdownTrigger::TerminationSignal,
        })
        .unwrap();
    assert!(late.joined);
    assert_eq!(late.token, token);
    assert_eq!(
        rig.prepare.release(&token),
        Ok(ReleaseOutcome::ExecutionUnderway)
    );
    rig.cancellation.gate.notify_one();
    let first = first.await.unwrap().unwrap();
    let second = second.await.unwrap().unwrap();
    assert_eq!(first, second);
    let third = rig.execute.execute(&token).await.unwrap();
    assert_eq!(third, first);
    assert_eq!(rig.cancellation.calls.load(Ordering::SeqCst), 1);
    assert_eq!(rig.routing.calls().len(), 2);
    assert_eq!(rig.persistence.calls.lock().unwrap().len(), 1);
    assert_eq!(rig.exit.signalled.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn simultaneous_os_parent_and_protocol_triggers_converge_on_one_outcome() {
    let rig = rig();
    let requests = [
        PrepareShutdownRequest {
            reason: ShutdownReason::TerminationSignal,
            trigger: ShutdownTrigger::TerminationSignal,
        },
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
    assert!(tokens.iter().all(|token| token == &tokens[0]));
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
    assert_eq!(rig.cancellation.calls.load(Ordering::SeqCst), 1);
    assert_eq!(rig.routing.calls().len(), 2);
    assert_eq!(rig.persistence.calls.lock().unwrap().len(), 1);
    assert_eq!(rig.exit.signalled.lock().unwrap().len(), 1);
    // After completion, a fresh prepare still joins: the harness is gone.
    let post = rig
        .prepare
        .execute(protocol(ShutdownReason::OperatorRequest))
        .unwrap();
    assert!(post.joined);
    assert_eq!(post.reason, ShutdownReason::TerminationSignal);
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
    assert!(rig.exit.signalled.lock().unwrap().is_empty());
}

#[test]
fn terminal_error_vocabulary_is_stable() {
    assert_eq!(
        HarnessShutdownError::NotPrepared.to_string(),
        "no shutdown has been prepared"
    );
    assert_eq!(
        HarnessShutdownError::UnknownToken.to_string(),
        "shutdown token is not the admitted one"
    );
    assert_eq!(
        HarnessShutdownError::AlreadyTerminated.to_string(),
        "harness already terminated"
    );
    assert_eq!(
        HarnessShutdownError::LifecycleViolation("x".into()).to_string(),
        "lifecycle violation: x"
    );
    assert_eq!(
        HarnessShutdownError::ExecutionInterrupted.to_string(),
        "shutdown execution was interrupted"
    );
}

#[tokio::test]
async fn a_cancelled_runner_hands_the_admission_back_and_releases_joiners() {
    let rig = rig_with(FakeCancellation::holding());
    let token = rig
        .prepare
        .execute(protocol(ShutdownReason::ParentShutdown))
        .unwrap()
        .token;
    let runner = tokio::spawn({
        let execute = rig.execute.clone();
        let token = token.clone();
        async move { execute.execute(&token).await }
    });
    while rig.cancellation.calls.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    let joiner = tokio::spawn({
        let execute = rig.execute.clone();
        let token = token.clone();
        async move { execute.execute(&token).await }
    });
    tokio::task::yield_now().await;
    runner.abort();
    assert!(runner.await.unwrap_err().is_cancelled());
    assert_eq!(
        joiner.await.unwrap(),
        Err(HarnessShutdownError::ExecutionInterrupted)
    );
    // Nothing past cancellation ran, the freeze held, and the same token
    // still runs the teardown to completion.
    assert!(rig.routing.calls().is_empty());
    assert_eq!(rig.lifecycle.lifecycle(), HarnessLifecycleState::Frozen);
    assert_eq!(rig.prepare.release(&token), Ok(ReleaseOutcome::Released));
    assert_eq!(rig.lifecycle.lifecycle(), HarnessLifecycleState::Accepting);
    let fresh = rig
        .prepare
        .execute(protocol(ShutdownReason::ParentShutdown))
        .unwrap()
        .token;
    rig.cancellation.hold.store(false, Ordering::SeqCst);
    let outcome = rig.execute.execute(&fresh).await.unwrap();
    assert_eq!(outcome.children_shut_down.len(), 2);
    assert_eq!(rig.exit.signalled.lock().unwrap().len(), 1);
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
