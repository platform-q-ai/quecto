use std::sync::Arc;

use super::*;
use crate::application::subagents::dto::ObserveOwnedChildExitRequest;
use crate::application::subagents::ports::CompensationObservation;
use crate::domain::ids::AgentUuid;

use super::super::lifecycle_fakes::*;
use super::super::teardown_fakes::identity;

fn rig(registry: Arc<FakeRegistry>) -> (Arc<FakeCompensation>, ObserveOwnedChildExit) {
    let compensation = FakeCompensation::new(registry.clone());
    (
        compensation.clone(),
        ObserveOwnedChildExit::new(registry, compensation),
    )
}

fn observe(uuid: &str, observation: ExitObservation) -> ObserveOwnedChildExitRequest {
    ObserveOwnedChildExitRequest {
        child: identity(uuid, 1),
        observation,
    }
}

#[tokio::test]
async fn a_reaped_exit_claims_and_compensates_once() {
    let registry = FakeRegistry::new()
        .with_row("A", 1, "alpha")
        .holding_process("A");
    let (compensation, use_case) = rig(registry.clone());
    let first = use_case
        .execute(observe("A", ExitObservation::ProcessExited))
        .await;
    assert_eq!(
        first,
        ObservedExit::Compensated {
            removed: vec![AgentUuid::new("A")]
        }
    );
    assert_eq!(
        compensation.calls(),
        [(
            identity("A", 1),
            TerminationCause::Exit(ExitObservation::ProcessExited)
        )]
    );
    // The monitor's EOF for the same child arrives later and joins.
    let second = use_case
        .execute(observe("A", ExitObservation::ConnectionClosed))
        .await;
    assert_eq!(
        second,
        ObservedExit::Joined(CompensationObservation::Compensated)
    );
    assert_eq!(compensation.calls().len(), 1, "compensated exactly once");
}

#[tokio::test]
async fn a_connection_observation_of_a_retained_process_defers_to_the_reaper() {
    let registry = FakeRegistry::new()
        .with_row("A", 1, "alpha")
        .holding_process("A");
    let (compensation, use_case) = rig(registry.clone());
    for observation in [
        ExitObservation::ConnectionClosed,
        ExitObservation::NeverReachable,
    ] {
        assert_eq!(
            use_case.execute(observe("A", observation)).await,
            ObservedExit::DeferredToProcessExit
        );
    }
    assert!(compensation.calls().is_empty(), "the row stays live");
    assert_eq!(registry.phase("A"), Phase::Live);
}

#[tokio::test]
async fn a_connection_observation_of_an_unowned_child_compensates() {
    let registry = FakeRegistry::new().with_row("S", 1, "script");
    let (compensation, use_case) = rig(registry.clone());
    let observed = use_case
        .execute(observe("S", ExitObservation::NeverReachable))
        .await;
    assert!(matches!(observed, ObservedExit::Compensated { .. }));
    assert_eq!(
        compensation.calls(),
        [(
            identity("S", 1),
            TerminationCause::Exit(ExitObservation::NeverReachable)
        )]
    );
}

#[tokio::test]
async fn an_exit_observed_while_a_kill_is_concluding_claims_the_compensation() {
    // The kill claimed the row stopping; the reaper observes the exit first
    // and owns the terminal effects, which the kill then joins.
    let registry = FakeRegistry::new().with_row("A", 1, "alpha");
    registry.set_phase("A", Phase::Stopping(TerminationCause::SelectedTermination));
    let (compensation, use_case) = rig(registry.clone());
    let observed = use_case
        .execute(observe("A", ExitObservation::ProcessExited))
        .await;
    assert!(matches!(observed, ObservedExit::Compensated { .. }));
    assert_eq!(compensation.calls().len(), 1);
    assert_eq!(registry.phase("A"), Phase::Compensated);
}

#[tokio::test]
async fn an_exit_of_an_unknown_row_joins_nothing() {
    let registry = FakeRegistry::new();
    let (compensation, use_case) = rig(registry);
    let observed = use_case
        .execute(observe("ghost", ExitObservation::ProcessExited))
        .await;
    assert_eq!(
        observed,
        ObservedExit::Joined(CompensationObservation::Unknown)
    );
    assert!(compensation.calls().is_empty());
}
