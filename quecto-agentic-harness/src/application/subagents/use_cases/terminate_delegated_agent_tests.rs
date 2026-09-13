use std::sync::Arc;

use super::*;
use crate::application::subagents::dto::{
    TerminateDelegatedAgentError, TerminateDelegatedAgentRequest, TerminationRouted,
};
use crate::domain::ids::AgentUuid;
use crate::domain::subagent_teardown::{
    HarnessLifecycleState, LaunchGeneration, LineageSnapshot, RoutingDepth, ShutdownReason,
    TerminationRouteError,
};

use super::super::teardown_fakes::*;

fn depth(hops: u32) -> RoutingDepth {
    RoutingDepth::new(hops).unwrap()
}

fn request(uuid: &str, generation: u64, hops: u32) -> TerminateDelegatedAgentRequest {
    TerminateDelegatedAgentRequest {
        target: identity(uuid, generation),
        remaining_depth: depth(hops),
    }
}

fn use_case(
    lineage: LineageSnapshot,
) -> (
    Arc<FakeLifecycle>,
    Arc<FakeRouting>,
    TerminateDelegatedAgent,
) {
    let lifecycle = FakeLifecycle::new(lineage);
    let routing = FakeRouting::new();
    let use_case = TerminateDelegatedAgent::new(lifecycle.clone(), routing.clone());
    (lifecycle, routing, use_case)
}

#[tokio::test]
async fn targeting_a_direct_child_invokes_self_shutdown_on_that_child_only() {
    let (_, routing, use_case) = use_case(root_tree());
    let routed = use_case.execute(request("A", 1, 1)).await.unwrap();
    assert_eq!(
        routed,
        TerminationRouted::ShutdownRequested {
            child: identity("A", 1)
        }
    );
    assert_eq!(
        routing.calls(),
        [RoutingCall::Shutdown(
            identity("A", 1),
            ShutdownReason::SelectedTermination
        )]
    );
}

#[tokio::test]
async fn targeting_a_grandchild_forwards_through_the_intermediate_and_keeps_it_alive() {
    let (lifecycle, routing, use_case) = use_case(root_tree());
    let routed = use_case.execute(request("B", 1, 3)).await.unwrap();
    assert_eq!(
        routed,
        TerminationRouted::Forwarded {
            via: identity("A", 1),
            remaining_depth: depth(2),
        }
    );
    assert_eq!(
        routing.calls(),
        [RoutingCall::Forward {
            via: identity("A", 1),
            target: identity("B", 1),
            remaining_depth: depth(2),
        }]
    );
    // The intermediate (A) and its unrelated child (C) were never shut down
    // and this harness is still accepting work.
    assert!(
        !routing
            .calls()
            .iter()
            .any(|call| matches!(call, RoutingCall::Shutdown(..)))
    );
    assert_eq!(lifecycle.lifecycle(), HarnessLifecycleState::Accepting);
    assert!(lifecycle.transitions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn intermediate_hop_shuts_down_the_direct_child_that_is_the_target() {
    // Now act as A: A → B, A → C. The forwarded command targets B with the
    // depth root left it.
    let a_view = LineageSnapshot {
        owner: AgentUuid::new("A"),
        records: vec![record("B", 1, "A"), record("C", 1, "A")],
    };
    let (_, routing, use_case) = use_case(a_view);
    let routed = use_case.execute(request("B", 1, 2)).await.unwrap();
    assert_eq!(
        routed,
        TerminationRouted::ShutdownRequested {
            child: identity("B", 1)
        }
    );
    assert_eq!(
        routing.calls(),
        [RoutingCall::Shutdown(
            identity("B", 1),
            ShutdownReason::SelectedTermination
        )]
    );
}

#[tokio::test]
async fn rejected_routes_touch_no_port() {
    let cases = [
        (
            request("B", 1, 1),
            TerminationRouteError::DepthExhausted {
                target: AgentUuid::new("B"),
                remaining: depth(1),
            },
        ),
        (
            request("A", 2, 1),
            TerminationRouteError::StaleGeneration {
                target: AgentUuid::new("A"),
                requested: LaunchGeneration::new(2),
                current: LaunchGeneration::new(1),
            },
        ),
        (
            request("nope", 1, 1),
            TerminationRouteError::UnknownTarget(AgentUuid::new("nope")),
        ),
        (request("root", 1, 1), TerminationRouteError::TargetIsSelf),
    ];
    for (request, expected) in cases {
        let (_, routing, use_case) = use_case(root_tree());
        assert_eq!(
            use_case.execute(request).await,
            Err(TerminateDelegatedAgentError::Rejected(expected))
        );
        assert!(routing.calls().is_empty());
    }
}

#[tokio::test]
async fn cyclic_lineage_is_rejected_without_effect() {
    let cyclic = LineageSnapshot {
        owner: AgentUuid::new("root"),
        records: vec![record("X", 1, "Y"), record("Y", 1, "X")],
    };
    let (_, routing, use_case) = use_case(cyclic);
    assert_eq!(
        use_case.execute(request("Y", 1, 8)).await,
        Err(TerminateDelegatedAgentError::Rejected(
            TerminationRouteError::LineageCycle(AgentUuid::new("Y"))
        ))
    );
    assert!(routing.calls().is_empty());
}

#[tokio::test]
async fn a_frozen_or_terminated_receiver_refuses_routing() {
    for state in [
        HarnessLifecycleState::Frozen,
        HarnessLifecycleState::Terminated,
    ] {
        let (lifecycle, routing, use_case) = use_case(root_tree());
        lifecycle.force(state);
        assert_eq!(
            use_case.execute(request("A", 1, 1)).await,
            Err(TerminateDelegatedAgentError::NotAccepting)
        );
        assert!(routing.calls().is_empty());
    }
}

#[tokio::test]
async fn an_unreachable_child_is_reported_in_capability_vocabulary() {
    let (_, routing, use_case) = use_case(root_tree());
    routing
        .unreachable
        .lock()
        .unwrap()
        .push(AgentUuid::new("A"));
    let error = use_case.execute(request("B", 1, 2)).await.unwrap_err();
    assert_eq!(
        error,
        TerminateDelegatedAgentError::ChildUnreachable {
            child: AgentUuid::new("A"),
            detail: "unreachable: socket closed".into(),
        }
    );
    assert_eq!(
        error.to_string(),
        "direct child A unreachable: unreachable: socket closed"
    );
    assert_eq!(
        TerminateDelegatedAgentError::NotAccepting.to_string(),
        "harness is not accepting control commands"
    );
    assert_eq!(
        TerminateDelegatedAgentError::Rejected(TerminationRouteError::TargetIsSelf).to_string(),
        "termination rejected: target is the receiving harness; use shutdown"
    );
}

/// The receiver of a selected termination claims the target stopping with
/// that intent before its edge (#1936 review): its own reaper then runs the
/// kill's compensation — no post-mortem, no "exited unexpectedly" note —
/// and a delivery failure lifts only the claim this edge took.
#[tokio::test]
async fn the_receiver_claims_the_target_stopping_before_routing_its_edge() {
    use super::super::lifecycle_fakes::{FakeRegistry, Phase};
    use crate::application::subagents::ports::TerminationCause;
    let registry = FakeRegistry::new()
        .with_row("A", 1, "alpha")
        .with_row("B", 1, "bravo")
        .with_row("D", 1, "delta");
    let lifecycle = FakeLifecycle::new(root_tree());
    let routing = FakeRouting::new();
    let use_case = TerminateDelegatedAgent::new(lifecycle.clone(), routing.clone())
        .with_registry(registry.clone());
    // A direct child: claimed, then shut down.
    use_case.execute(request("D", 1, 1)).await.unwrap();
    assert_eq!(
        registry.phase("D"),
        Phase::Stopping(TerminationCause::SelectedTermination)
    );
    assert_eq!(
        registry.trace().last().map(String::as_str),
        Some("claim-stopping D")
    );
    // A nested target: its reported row is claimed too, the ancestor's is not.
    use_case.execute(request("B", 1, 3)).await.unwrap();
    assert_eq!(
        registry.phase("B"),
        Phase::Stopping(TerminationCause::SelectedTermination)
    );
    assert_eq!(registry.phase("A"), Phase::Live);
    // An edge that cannot be delivered lifts the claim this edge took...
    let registry = FakeRegistry::new().with_row("A", 1, "alpha");
    let use_case = TerminateDelegatedAgent::new(lifecycle.clone(), routing.clone())
        .with_registry(registry.clone());
    routing
        .unreachable
        .lock()
        .unwrap()
        .push(AgentUuid::new("A"));
    use_case.execute(request("A", 1, 1)).await.unwrap_err();
    assert_eq!(registry.phase("A"), Phase::Live);
    assert_eq!(registry.trace(), ["claim-stopping A", "release-stopping A"]);
    // ...but never a claim another path (an operator kill) already holds.
    registry.set_phase("A", Phase::Stopping(TerminationCause::SelectedTermination));
    use_case.execute(request("A", 1, 1)).await.unwrap_err();
    assert_eq!(
        registry.phase("A"),
        Phase::Stopping(TerminationCause::SelectedTermination)
    );
    // A refused route claims nothing.
    let registry = FakeRegistry::new().with_row("A", 9, "alpha");
    let use_case = TerminateDelegatedAgent::new(lifecycle, routing).with_registry(registry.clone());
    use_case.execute(request("A", 9, 1)).await.unwrap_err();
    assert!(registry.trace().is_empty());
}
