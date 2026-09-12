use std::sync::Arc;

use super::*;
use crate::application::subagents::dto::CompensateFailedLaunchRequest;
use crate::application::subagents::ports::TerminationConclusion;
use crate::domain::ids::AgentUuid;

use super::super::lifecycle_fakes::*;
use super::super::teardown_fakes::{FakeRouting, RoutingCall, identity};

struct Rig {
    registry: Arc<FakeRegistry>,
    routing: Arc<FakeRouting>,
    termination: Arc<FakeTermination>,
    compensation: Arc<FakeCompensation>,
    use_case: CompensateFailedLaunch,
}

fn rig(registry: Arc<FakeRegistry>, conclusion: TerminationConclusion) -> Rig {
    let routing = FakeRouting::new();
    let termination = FakeTermination::new(registry.clone(), conclusion);
    let compensation = FakeCompensation::new(registry.clone());
    let use_case = CompensateFailedLaunch::new(CompensateFailedLaunchPorts {
        registry: registry.clone(),
        routing: routing.clone(),
        termination: termination.clone(),
        compensation: compensation.clone(),
    });
    Rig {
        registry,
        routing,
        termination,
        compensation,
        use_case,
    }
}

fn request(uuid: &str, owns_environment: bool) -> CompensateFailedLaunchRequest {
    CompensateFailedLaunchRequest {
        child: identity(uuid, 1),
        owns_environment,
    }
}

#[tokio::test]
async fn a_registered_launch_is_asked_to_shut_down_then_concluded_then_rolled_back() {
    let registry = FakeRegistry::new()
        .with_row("A", 1, "alpha")
        .holding_process("A");
    let rig = rig(registry, TerminationConclusion::ExitedAfterProtocol);
    let outcome = rig.use_case.execute(request("A", true)).await;
    assert_eq!(
        outcome.conclusion,
        TerminationConclusion::ExitedAfterProtocol
    );
    assert_eq!(outcome.removed, [AgentUuid::new("A")]);
    assert_eq!(
        rig.routing.calls(),
        [RoutingCall::Shutdown(
            identity("A", 1),
            ShutdownReason::OperatorRequest
        )]
    );
    assert_eq!(
        rig.termination.calls(),
        [(
            identity("A", 1),
            ProtocolAttempt::Acknowledged,
            ConclusionBudget::Rollback
        )]
    );
    assert_eq!(
        rig.compensation.calls(),
        [(
            identity("A", 1),
            TerminationCause::LaunchRollback {
                owns_environment: true
            }
        )]
    );
    assert_eq!(
        rig.registry.trace(),
        ["claim-stopping A", "claim-terminal A", "compensated A"]
    );
}

#[tokio::test]
async fn an_unreachable_child_hands_the_negative_outcome_to_the_fallback() {
    let registry = FakeRegistry::new()
        .with_row("A", 1, "alpha")
        .holding_process("A");
    let rig = rig(registry, TerminationConclusion::ExitedAfterFallback);
    rig.routing
        .unreachable
        .lock()
        .unwrap()
        .push(AgentUuid::new("A"));
    let outcome = rig.use_case.execute(request("A", false)).await;
    assert_eq!(
        outcome.conclusion,
        TerminationConclusion::ExitedAfterFallback
    );
    assert_eq!(
        rig.termination.calls()[0].1,
        ProtocolAttempt::Negative("unreachable: socket closed".into())
    );
    assert_eq!(
        rig.compensation.calls()[0].1,
        TerminationCause::LaunchRollback {
            owns_environment: false
        }
    );
}

#[tokio::test]
async fn a_script_managed_member_is_compensated_without_waiting_for_its_exit() {
    let registry = FakeRegistry::new().with_row("S", 1, "script");
    let rig = rig(registry, TerminationConclusion::NoRetainedHandle);
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        rig.use_case.execute(request("S", false)),
    )
    .await
    .expect("a rollback never waits on a child it does not own");
    assert_eq!(outcome.conclusion, TerminationConclusion::NoRetainedHandle);
    assert_eq!(rig.compensation.calls().len(), 1);
    assert_eq!(rig.registry.phase("S"), Phase::Compensated);
}

#[tokio::test]
async fn a_rollback_of_a_row_already_being_ended_joins_without_a_second_edge() {
    let registry = FakeRegistry::new().with_row("A", 1, "alpha");
    registry.set_phase("A", Phase::Compensated);
    let rig = rig(registry, TerminationConclusion::ExitedAfterProtocol);
    let outcome = rig.use_case.execute(request("A", false)).await;
    assert_eq!(outcome.conclusion, TerminationConclusion::NoRetainedHandle);
    assert!(outcome.removed.is_empty());
    assert!(rig.routing.calls().is_empty());
    assert!(rig.termination.calls().is_empty());
    assert!(rig.compensation.calls().is_empty());
}

#[tokio::test]
async fn a_rollback_of_a_never_registered_child_does_nothing() {
    let rig = rig(
        FakeRegistry::new(),
        TerminationConclusion::ExitedAfterProtocol,
    );
    let outcome = rig.use_case.execute(request("never", false)).await;
    assert_eq!(outcome.conclusion, TerminationConclusion::NoRetainedHandle);
    assert!(rig.routing.calls().is_empty());
    assert!(rig.compensation.calls().is_empty());
}

/// A rollback that finds another termination in flight joins it and never
/// compensates a row whose process may still be alive (review: the kill
/// owns the row; only its observed exit reaches the terminal effects).
#[tokio::test]
async fn a_rollback_racing_a_kill_joins_and_never_claims_the_terminal_effects() {
    let registry = FakeRegistry::new()
        .with_row("A", 1, "alpha")
        .holding_process("A");
    registry.set_phase("A", Phase::Stopping(TerminationCause::SelectedTermination));
    let rig = rig(registry.clone(), TerminationConclusion::ExitedAfterProtocol);
    let joiner = {
        let use_case =
            std::sync::Arc::new(CompensateFailedLaunch::new(CompensateFailedLaunchPorts {
                registry: registry.clone(),
                routing: rig.routing.clone(),
                termination: rig.termination.clone(),
                compensation: rig.compensation.clone(),
            }));
        tokio::spawn(async move { use_case.execute(request("A", false)).await })
    };
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(
        registry.phase("A"),
        Phase::Stopping(TerminationCause::SelectedTermination),
        "the rollback neither claimed nor compensated while the kill owns the row"
    );
    assert!(rig.compensation.calls().is_empty());
    // The kill finishes: the joiner returns with nothing of its own.
    registry.set_phase("A", Phase::Compensated);
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(2), joiner)
        .await
        .expect("the joiner returns once the owner compensates")
        .unwrap();
    assert_eq!(outcome.conclusion, TerminationConclusion::NoRetainedHandle);
    assert!(outcome.removed.is_empty());
    assert!(rig.routing.calls().is_empty());
    assert!(rig.termination.calls().is_empty());
}

/// Two rollbacks of the same launch, concurrently: exactly one ends the
/// child and compensates; the other joins.
#[tokio::test]
async fn concurrent_rollbacks_compensate_exactly_once() {
    let registry = FakeRegistry::new().with_row("A", 1, "alpha");
    let compensation = FakeCompensation::new(registry.clone());
    let use_case = std::sync::Arc::new(CompensateFailedLaunch::new(CompensateFailedLaunchPorts {
        registry: registry.clone(),
        routing: FakeRouting::new(),
        termination: FakeTermination::new(
            registry.clone(),
            TerminationConclusion::NoRetainedHandle,
        ),
        compensation: compensation.clone(),
    }));
    let mut tasks = Vec::new();
    for _ in 0..2 {
        let use_case = use_case.clone();
        tasks.push(tokio::spawn(async move {
            use_case.execute(request("A", false)).await
        }));
    }
    let mut removed_total = 0;
    for task in tasks {
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(2), task)
            .await
            .expect("bounded")
            .unwrap();
        removed_total += outcome.removed.len();
    }
    assert_eq!(removed_total, 1, "one rollback removed the row");
    assert_eq!(compensation.calls().len(), 1);
    assert_eq!(registry.phase("A"), Phase::Compensated);
}
