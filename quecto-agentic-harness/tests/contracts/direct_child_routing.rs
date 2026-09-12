//! Contract for [`DirectChildRouting`] (#1934): the port carries exactly one
//! edge of control, and the selected-routing use case proves root→A→B
//! targeting B forwards via A (A and its unrelated children untouched),
//! targeting A invokes self shutdown on A, and refused routes touch no edge.
use std::sync::Arc;

use quecto::application::subagents::dto::{
    TerminateDelegatedAgentError, TerminateDelegatedAgentRequest, TerminationRouted,
};
use quecto::application::subagents::ports::{ChildRoutingError, DirectChildRouting};
use quecto::application::subagents::use_cases::TerminateDelegatedAgent;
use quecto::domain::ids::AgentUuid;
use quecto::domain::subagent_teardown::{
    LaunchGeneration, LineageSnapshot, RoutingDepth, ShutdownReason, TerminationRouteError,
};

use super::teardown_fixture::{Call, Lifecycle, Routing, identity, record, root_tree};

fn depth(hops: u32) -> RoutingDepth {
    RoutingDepth::new(hops).unwrap()
}

fn request(uuid: &str, generation: u64, hops: u32) -> TerminateDelegatedAgentRequest {
    TerminateDelegatedAgentRequest {
        target: identity(uuid, generation),
        remaining_depth: depth(hops),
    }
}

fn routed_use_case(lineage: LineageSnapshot) -> (Arc<Routing>, TerminateDelegatedAgent) {
    let routing = Routing::new();
    let port: Arc<dyn DirectChildRouting> = routing.clone();
    (
        routing,
        TerminateDelegatedAgent::new(Lifecycle::new(lineage), port),
    )
}

#[tokio::test]
async fn port_is_object_safe_and_send_and_reports_in_its_own_vocabulary() {
    let routing = Routing::new();
    routing
        .unreachable
        .lock()
        .unwrap()
        .push(AgentUuid::new("A"));
    let port: Arc<dyn DirectChildRouting + Send + Sync> = routing.clone();
    let child = identity("A", 1);
    let outcome = tokio::spawn(async move {
        port.shutdown_child(&child, ShutdownReason::ParentShutdown)
            .await
    })
    .await
    .unwrap();
    assert_eq!(
        outcome,
        Err(ChildRoutingError::Unreachable("peer gone".into()))
    );
    assert_eq!(
        ChildRoutingError::NotADirectChild.to_string(),
        "not a direct child of this harness"
    );
    assert_eq!(
        ChildRoutingError::Unreachable("x".into()).to_string(),
        "unreachable: x"
    );
}

#[tokio::test]
async fn root_targeting_b_forwards_via_a_and_never_shuts_a_or_its_siblings_down() {
    let (routing, use_case) = routed_use_case(root_tree());
    let routed = use_case.execute(request("B", 1, 2)).await.unwrap();
    assert_eq!(
        routed,
        TerminationRouted::Forwarded {
            via: identity("A", 1),
            remaining_depth: depth(1),
        }
    );
    assert_eq!(
        routing.calls(),
        [Call::Forward {
            via: identity("A", 1),
            target: identity("B", 1),
            remaining_depth: depth(1),
        }]
    );
    // Now A receives the forwarded command with the remaining budget and
    // shuts down B only; C (A's unrelated child) is never addressed.
    let a_view = LineageSnapshot {
        owner: AgentUuid::new("A"),
        records: vec![record("B", 1, "A"), record("C", 1, "A")],
    };
    let (a_routing, a_use_case) = routed_use_case(a_view);
    let routed = a_use_case.execute(request("B", 1, 1)).await.unwrap();
    assert_eq!(
        routed,
        TerminationRouted::ShutdownRequested {
            child: identity("B", 1)
        }
    );
    assert_eq!(
        a_routing.calls(),
        [Call::Shutdown(
            identity("B", 1),
            ShutdownReason::SelectedTermination
        )]
    );
}

#[tokio::test]
async fn root_targeting_a_uses_self_shutdown_on_a() {
    let (routing, use_case) = routed_use_case(root_tree());
    let routed = use_case.execute(request("A", 1, 3)).await.unwrap();
    assert_eq!(
        routed,
        TerminationRouted::ShutdownRequested {
            child: identity("A", 1)
        }
    );
    assert_eq!(
        routing.calls(),
        [Call::Shutdown(
            identity("A", 1),
            ShutdownReason::SelectedTermination
        )]
    );
}

#[tokio::test]
async fn stale_cyclic_and_over_depth_routes_touch_no_edge() {
    let cyclic = LineageSnapshot {
        owner: AgentUuid::new("root"),
        records: vec![record("X", 1, "Y"), record("Y", 1, "X")],
    };
    let cases = [
        (
            root_tree(),
            request("A", 2, 1),
            TerminationRouteError::StaleGeneration {
                target: AgentUuid::new("A"),
                requested: LaunchGeneration::new(2),
                current: LaunchGeneration::new(1),
            },
        ),
        (
            root_tree(),
            request("B", 1, 1),
            TerminationRouteError::DepthExhausted {
                target: AgentUuid::new("B"),
                remaining: depth(1),
            },
        ),
        (
            cyclic,
            request("X", 1, 8),
            TerminationRouteError::LineageCycle(AgentUuid::new("X")),
        ),
        (
            root_tree(),
            request("root", 1, 1),
            TerminationRouteError::TargetIsSelf,
        ),
        (
            root_tree(),
            request("ghost", 1, 1),
            TerminationRouteError::UnknownTarget(AgentUuid::new("ghost")),
        ),
    ];
    for (lineage, request, expected) in cases {
        let (routing, use_case) = routed_use_case(lineage);
        assert_eq!(
            use_case.execute(request).await,
            Err(TerminateDelegatedAgentError::Rejected(expected))
        );
        assert!(routing.calls().is_empty());
    }
}
