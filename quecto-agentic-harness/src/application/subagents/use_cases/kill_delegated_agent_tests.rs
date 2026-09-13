use std::sync::Arc;

use super::*;
use crate::application::subagents::dto::{KillDelegatedAgentError, KillDelegatedAgentRequest};
use crate::application::subagents::ports::{ResolutionError, TerminationConclusion};
use crate::domain::ids::AgentUuid;
use crate::domain::subagent_teardown::{
    HarnessLifecycleState, LineageSnapshot, ShutdownReason, TerminationRouteError,
};

use super::super::lifecycle_fakes::*;
use super::super::teardown_fakes::*;

struct Rig {
    registry: Arc<FakeRegistry>,
    routing: Arc<FakeRouting>,
    lifecycle: Arc<FakeLifecycle>,
    termination: Arc<FakeTermination>,
    compensation: Arc<FakeCompensation>,
    use_case: KillDelegatedAgent,
}

fn rig(
    registry: Arc<FakeRegistry>,
    lineage: LineageSnapshot,
    conclusion: TerminationConclusion,
) -> Rig {
    let routing = FakeRouting::new();
    let lifecycle = FakeLifecycle::new(lineage);
    let termination = FakeTermination::new(registry.clone(), conclusion);
    let compensation = FakeCompensation::new(registry.clone());
    let route = Arc::new(TerminateDelegatedAgent::new(
        lifecycle.clone(),
        routing.clone(),
    ));
    let use_case = KillDelegatedAgent::new(
        route,
        KillDelegatedAgentPorts {
            registry: registry.clone(),
            lifecycle: lifecycle.clone(),
            termination: termination.clone(),
            compensation: compensation.clone(),
        },
    );
    Rig {
        registry,
        routing,
        lifecycle,
        termination,
        compensation,
        use_case,
    }
}

/// Rows for the fake tree: A and D are direct children (owned), B and C
/// are A's children reported upward.
fn tree_registry() -> Arc<FakeRegistry> {
    FakeRegistry::new()
        .with_row("A", 1, "alpha")
        .holding_process("A")
        .with_row("B", 1, "bravo")
        .with_row("C", 1, "charlie")
        .with_row("D", 1, "delta")
        .holding_process("D")
}

fn kill(reference: &str) -> KillDelegatedAgentRequest {
    KillDelegatedAgentRequest {
        reference: reference.to_owned(),
    }
}

async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(std::time::Duration::from_secs(5), future)
        .await
        .expect("the use case must settle within the bound")
}

#[tokio::test]
async fn direct_child_is_claimed_then_shut_down_then_compensated_gracefully() {
    let rig = rig(
        tree_registry(),
        root_tree(),
        TerminationConclusion::ExitedAfterProtocol,
    );
    let outcome = bounded(rig.use_case.execute(kill("A"))).await.unwrap();
    assert_eq!(outcome.target, identity("A", 1));
    assert_eq!(outcome.result, TerminationResult::Graceful);
    assert_eq!(outcome.removed, [AgentUuid::new("A")]);
    // One edge, self shutdown, to the target only.
    assert_eq!(
        rig.routing.calls(),
        [RoutingCall::Shutdown(
            identity("A", 1),
            ShutdownReason::SelectedTermination
        )]
    );
    // The fallback was handed the acknowledgement, never a negative outcome.
    assert_eq!(
        rig.termination.calls(),
        [(
            identity("A", 1),
            ProtocolAttempt::Acknowledged,
            ConclusionBudget::Standard
        )]
    );
    assert_eq!(
        rig.compensation.calls(),
        [(identity("A", 1), TerminationCause::SelectedTermination)]
    );
    // Claimed before the edge, compensated only after the exit.
    assert_eq!(
        rig.registry.trace(),
        ["claim-stopping A", "claim-terminal A", "compensated A"]
    );
    assert_eq!(rig.registry.phase("A"), Phase::Compensated);
    assert_eq!(rig.lifecycle.lifecycle(), HarnessLifecycleState::Accepting);
}

#[tokio::test]
async fn nested_target_is_forwarded_and_its_ancestor_never_gets_self_shutdown() {
    let mut lineage = root_tree();
    lineage.records.push(record("E", 1, "B"));
    let registry = tree_registry().with_row("E", 1, "echo");
    let rig = rig(registry, lineage, TerminationConclusion::NoRetainedHandle);
    let registry = rig.registry.clone();
    // The intermediate's reported snapshot later drops B: its row (and
    // E's) is compensated by the merge, which is what the kill observes.
    let observer = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        registry.set_phase("B", Phase::Compensated);
    });
    let outcome = bounded(rig.use_case.execute(kill("B"))).await.unwrap();
    bounded(observer).await.unwrap();
    assert_eq!(outcome.result, TerminationResult::Graceful);
    assert_eq!(outcome.removed, [AgentUuid::new("B"), AgentUuid::new("E")]);
    assert_eq!(
        rig.routing.calls(),
        [RoutingCall::Forward {
            via: identity("A", 1),
            target: identity("B", 1),
            remaining_depth: RoutingDepth::new(RoutingDepth::MAX_HOPS - 1).unwrap(),
        }]
    );
    assert!(
        rig.termination.calls().is_empty(),
        "no fallback for a target this harness does not own"
    );
    assert!(
        rig.compensation.calls().is_empty(),
        "the merge already compensated; nothing runs twice"
    );
    assert_eq!(
        rig.registry.phase("A"),
        Phase::Live,
        "the ancestor survives"
    );
    assert_eq!(rig.registry.phase("C"), Phase::Live, "its sibling survives");
}

#[tokio::test]
async fn ambiguous_display_label_has_no_effect() {
    let registry = tree_registry().with_row("X", 1, "alpha");
    let rig = rig(
        registry,
        root_tree(),
        TerminationConclusion::NoRetainedHandle,
    );
    let error = bounded(rig.use_case.execute(kill("alpha")))
        .await
        .unwrap_err();
    assert_eq!(
        error,
        KillDelegatedAgentError::Unresolved(ResolutionError::Ambiguous)
    );
    assert!(rig.routing.calls().is_empty());
    assert!(rig.registry.trace().is_empty());
}

#[tokio::test]
async fn display_label_resolves_to_the_one_live_row() {
    let rig = rig(
        tree_registry(),
        root_tree(),
        TerminationConclusion::ExitedAfterProtocol,
    );
    let outcome = bounded(rig.use_case.execute(kill("delta"))).await.unwrap();
    assert_eq!(outcome.target, identity("D", 1));
}

#[tokio::test]
async fn missing_dead_and_undelegated_targets_have_no_effect() {
    let registry = tree_registry()
        .with_row("F", 1, "foxtrot")
        .not_delegated("F");
    registry.set_phase("D", Phase::Compensated);
    let rig = rig(
        registry,
        root_tree(),
        TerminationConclusion::NoRetainedHandle,
    );
    for (reference, expected) in [
        ("ghost", ResolutionError::Unknown),
        ("D", ResolutionError::Exited),
        ("F", ResolutionError::NotDelegated),
    ] {
        let error = bounded(rig.use_case.execute(kill(reference)))
            .await
            .unwrap_err();
        assert_eq!(error, KillDelegatedAgentError::Unresolved(expected));
    }
    assert!(rig.routing.calls().is_empty());
    assert!(rig.registry.trace().is_empty());
}

#[tokio::test]
async fn stale_generation_is_refused_and_the_claim_lifted() {
    // The registry knows A at generation 2; the lineage the route sees
    // still carries generation 1.
    let registry = FakeRegistry::new().with_row("A", 2, "alpha");
    let rig = rig(
        registry,
        root_tree(),
        TerminationConclusion::NoRetainedHandle,
    );
    let error = bounded(rig.use_case.execute(kill("A"))).await.unwrap_err();
    assert!(matches!(
        error,
        KillDelegatedAgentError::Rejected(TerminationRouteError::StaleGeneration { .. })
    ));
    assert!(rig.routing.calls().is_empty(), "no edge for a stale target");
    assert_eq!(rig.registry.phase("A"), Phase::Live);
    assert_eq!(
        rig.registry.trace(),
        ["claim-stopping A", "release-stopping A"]
    );
}

#[tokio::test]
async fn cycles_and_over_depth_routes_are_refused_with_no_effect() {
    let mut cyclic = root_tree();
    cyclic.records.push(record("Y", 1, "Z"));
    cyclic.records.push(record("Z", 1, "Y"));
    let registry = tree_registry().with_row("Y", 1, "yankee");
    let rig = rig(registry, cyclic, TerminationConclusion::NoRetainedHandle);
    let error = bounded(rig.use_case.execute(kill("Y"))).await.unwrap_err();
    assert!(matches!(
        error,
        KillDelegatedAgentError::Rejected(TerminationRouteError::LineageCycle(_))
    ));
    assert_eq!(rig.registry.phase("Y"), Phase::Live);

    let mut deep = LineageSnapshot {
        owner: AgentUuid::new("root"),
        records: vec![record("n0", 1, "root")],
    };
    for hop in 1..=RoutingDepth::MAX_HOPS {
        deep.records
            .push(record(&format!("n{hop}"), 1, &format!("n{}", hop - 1)));
    }
    let leaf = format!("n{}", RoutingDepth::MAX_HOPS);
    let registry = FakeRegistry::new().with_row(&leaf, 1, "leaf");
    let deep_rig = self::rig(registry, deep, TerminationConclusion::NoRetainedHandle);
    let error = bounded(deep_rig.use_case.execute(kill(&leaf)))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        KillDelegatedAgentError::Rejected(TerminationRouteError::DepthExhausted { .. })
    ));
    assert!(deep_rig.routing.calls().is_empty());
    assert_eq!(deep_rig.registry.phase(&leaf), Phase::Live);
}

#[tokio::test]
async fn a_route_that_disappeared_between_resolution_and_routing_is_refused() {
    // The row exists but the lineage no longer lists it.
    let registry = FakeRegistry::new().with_row("Q", 1, "quebec");
    let rig = rig(
        registry,
        root_tree(),
        TerminationConclusion::NoRetainedHandle,
    );
    let error = bounded(rig.use_case.execute(kill("Q"))).await.unwrap_err();
    assert_eq!(
        error,
        KillDelegatedAgentError::Rejected(TerminationRouteError::UnknownTarget(AgentUuid::new(
            "Q"
        )))
    );
    assert_eq!(rig.registry.phase("Q"), Phase::Live);
    assert!(rig.routing.calls().is_empty());
}

#[tokio::test]
async fn an_unreachable_owned_child_falls_back_and_is_reported_as_fallback() {
    let rig = rig(
        tree_registry(),
        root_tree(),
        TerminationConclusion::ExitedAfterFallback,
    );
    rig.routing
        .unreachable
        .lock()
        .unwrap()
        .push(AgentUuid::new("A"));
    let outcome = bounded(rig.use_case.execute(kill("A"))).await.unwrap();
    assert_eq!(outcome.result, TerminationResult::Fallback);
    assert_eq!(
        rig.termination.calls(),
        [(
            identity("A", 1),
            ProtocolAttempt::Negative("unreachable: socket closed".into()),
            ConclusionBudget::Standard
        )]
    );
    assert_eq!(rig.compensation.calls().len(), 1);
}

#[tokio::test]
async fn an_already_exited_owned_child_is_reported_as_such() {
    let rig = rig(
        tree_registry(),
        root_tree(),
        TerminationConclusion::AlreadyExited,
    );
    let outcome = bounded(rig.use_case.execute(kill("D"))).await.unwrap();
    assert_eq!(outcome.result, TerminationResult::AlreadyExited);
}

#[tokio::test]
async fn an_unreachable_child_without_a_retained_handle_fails_truthfully() {
    let registry = FakeRegistry::new().with_row("A", 1, "alpha");
    let rig = rig(
        registry,
        root_tree(),
        TerminationConclusion::NoRetainedHandle,
    );
    rig.routing
        .unreachable
        .lock()
        .unwrap()
        .push(AgentUuid::new("A"));
    let error = bounded(rig.use_case.execute(kill("A"))).await.unwrap_err();
    assert_eq!(
        error,
        KillDelegatedAgentError::Failed {
            detail: "unreachable: socket closed".into()
        }
    );
    assert!(
        rig.compensation.calls().is_empty(),
        "no removal without an exit"
    );
    assert_eq!(rig.registry.phase("A"), Phase::Live, "the claim was lifted");
}

#[tokio::test]
async fn an_acknowledged_child_without_a_handle_whose_exit_is_not_observed_fails() {
    let registry = FakeRegistry::new().with_row("A", 1, "alpha");
    *registry.time_out_waits.lock().unwrap() = true;
    let rig = rig(
        registry,
        root_tree(),
        TerminationConclusion::NoRetainedHandle,
    );
    let error = bounded(rig.use_case.execute(kill("A"))).await.unwrap_err();
    assert!(matches!(error, KillDelegatedAgentError::Failed { .. }));
    assert_eq!(rig.registry.phase("A"), Phase::Live);
    assert!(rig.compensation.calls().is_empty());
}

#[tokio::test]
async fn a_fallback_that_could_not_end_the_child_fails_and_lifts_the_claim() {
    let rig = rig(
        tree_registry(),
        root_tree(),
        TerminationConclusion::StillRunning("kill sent; no exit within 2s".into()),
    );
    let error = bounded(rig.use_case.execute(kill("A"))).await.unwrap_err();
    assert_eq!(
        error,
        KillDelegatedAgentError::Failed {
            detail: "kill sent; no exit within 2s".into()
        }
    );
    assert_eq!(rig.registry.phase("A"), Phase::Live);
}

#[tokio::test]
async fn a_kill_racing_a_natural_exit_compensates_exactly_once() {
    let rig = rig(
        tree_registry(),
        root_tree(),
        TerminationConclusion::ExitedAfterProtocol,
    );
    *rig.termination.reaper_compensates.lock().unwrap() = true;
    let outcome = bounded(rig.use_case.execute(kill("A"))).await.unwrap();
    assert_eq!(outcome.result, TerminationResult::Graceful);
    // The kill joined the reaper's compensation and reports the subtree
    // the route saw beneath the target.
    assert_eq!(
        outcome.removed,
        [
            AgentUuid::new("A"),
            AgentUuid::new("B"),
            AgentUuid::new("C")
        ]
    );
    assert!(
        rig.compensation.calls().is_empty(),
        "the reaper compensated; the kill joined"
    );
    assert_eq!(
        rig.registry.trace(),
        [
            "claim-stopping A",
            "claim-terminal A",
            "reaper-compensated A",
            "await-compensated A"
        ]
    );
}

#[tokio::test]
async fn a_second_kill_of_a_stopping_agent_is_refused() {
    let registry = tree_registry();
    registry.set_phase("A", Phase::Stopping(TerminationCause::SelectedTermination));
    let rig = rig(
        registry,
        root_tree(),
        TerminationConclusion::NoRetainedHandle,
    );
    let error = bounded(rig.use_case.execute(kill("A"))).await.unwrap_err();
    assert_eq!(error, KillDelegatedAgentError::AlreadyStopping);
    assert!(rig.routing.calls().is_empty());
}

#[tokio::test]
async fn a_frozen_harness_refuses_and_lifts_the_claim() {
    let rig = rig(
        tree_registry(),
        root_tree(),
        TerminationConclusion::NoRetainedHandle,
    );
    rig.lifecycle.force(HarnessLifecycleState::Frozen);
    let error = bounded(rig.use_case.execute(kill("A"))).await.unwrap_err();
    assert_eq!(error, KillDelegatedAgentError::NotAccepting);
    assert_eq!(rig.registry.phase("A"), Phase::Live);
}

#[tokio::test]
async fn an_unreachable_intermediate_reports_the_route_not_the_target() {
    let rig = rig(
        tree_registry(),
        root_tree(),
        TerminationConclusion::NoRetainedHandle,
    );
    rig.routing
        .unreachable
        .lock()
        .unwrap()
        .push(AgentUuid::new("A"));
    let error = bounded(rig.use_case.execute(kill("B"))).await.unwrap_err();
    assert_eq!(
        error,
        KillDelegatedAgentError::RouteUnreachable {
            via: AgentUuid::new("A"),
            detail: "unreachable: socket closed".into()
        }
    );
    assert!(rig.termination.calls().is_empty());
    assert_eq!(rig.registry.phase("B"), Phase::Live);
    assert_eq!(rig.registry.phase("A"), Phase::Live);
}

#[test]
fn errors_render_their_vocabulary() {
    for (error, expected) in [
        (
            KillDelegatedAgentError::Unresolved(ResolutionError::Unknown),
            "not found in registry",
        ),
        (
            KillDelegatedAgentError::AlreadyStopping,
            "a termination is already in flight",
        ),
        (
            KillDelegatedAgentError::NotAccepting,
            "harness is not accepting control commands",
        ),
        (
            KillDelegatedAgentError::RouteUnreachable {
                via: AgentUuid::new("A"),
                detail: "gone".into(),
            },
            "route via A unreachable: gone",
        ),
        (
            KillDelegatedAgentError::Failed {
                detail: "no exit".into(),
            },
            "termination failed: no exit",
        ),
        (
            KillDelegatedAgentError::Rejected(TerminationRouteError::TargetIsSelf),
            "termination rejected: target is the receiving harness; use shutdown",
        ),
    ] {
        assert_eq!(error.to_string(), expected);
    }
    assert_eq!(TerminationResult::Graceful.to_string(), "graceful");
    assert_eq!(TerminationResult::Fallback.as_str(), "fallback");
    assert_eq!(TerminationResult::AlreadyExited.as_str(), "already-exited");
}

/// The child dies (and its reaper claims, compensates and retires the
/// handle) while the kill's protocol attempt is in flight, so the edge
/// answers negatively and no handle remains: that is an exited child with
/// this kill's intent already honoured, never a failed termination.
#[tokio::test]
async fn a_child_reaped_during_the_protocol_attempt_is_reported_as_already_exited() {
    let registry = tree_registry();
    let routing = FakeRouting::new();
    // The reaper wins while the edge is held open.
    *routing.hold_child.lock().unwrap() = Some(AgentUuid::new("A"));
    routing
        .unreachable
        .lock()
        .unwrap()
        .push(AgentUuid::new("A"));
    let lifecycle = FakeLifecycle::new(root_tree());
    let termination =
        FakeTermination::new(registry.clone(), TerminationConclusion::NoRetainedHandle);
    let compensation = FakeCompensation::new(registry.clone());
    let use_case = KillDelegatedAgent::new(
        Arc::new(TerminateDelegatedAgent::new(
            lifecycle.clone(),
            routing.clone(),
        )),
        KillDelegatedAgentPorts {
            registry: registry.clone(),
            lifecycle,
            termination: termination.clone(),
            compensation: compensation.clone(),
        },
    );
    let kill_task = {
        let registry = registry.clone();
        let routing = routing.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            // Reaper: claims the terminal effects (kill intent) and compensates.
            assert_eq!(
                registry.claim_terminal(&identity("A", 1)),
                TerminalClaim::Claimed
            );
            registry.set_phase("A", Phase::Compensated);
            // Now the edge answers: the socket is gone.
            routing.gate.notify_one();
        })
    };
    let outcome = bounded(use_case.execute(kill("A"))).await.unwrap();
    bounded(kill_task).await.unwrap();
    assert_eq!(outcome.result, TerminationResult::AlreadyExited);
    assert_eq!(
        outcome.removed,
        [
            AgentUuid::new("A"),
            AgentUuid::new("B"),
            AgentUuid::new("C")
        ]
    );
    assert!(
        compensation.calls().is_empty(),
        "the reaper's compensation was joined"
    );
    assert_eq!(registry.phase("A"), Phase::Compensated);
    assert_eq!(
        termination.calls()[0].1,
        ProtocolAttempt::Negative("unreachable: socket closed".into())
    );
}
