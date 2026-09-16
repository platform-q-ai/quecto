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
            child: identity("A", 1),
            result: None,
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
            result: None,
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
            child: identity("B", 1),
            result: None,
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
    // A nested target: its reported row is claimed for the hop and lifted
    // again once the hop answered (the owner's snapshot ends the row); the
    // ancestor's row is never claimed.
    use_case.execute(request("B", 1, 3)).await.unwrap();
    assert_eq!(registry.phase("B"), Phase::Live);
    assert_eq!(
        registry
            .trace()
            .iter()
            .rev()
            .take(2)
            .rev()
            .collect::<Vec<_>>(),
        ["claim-stopping B", "release-stopping B"]
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
    // ...but never a claim another path (an operator kill) already holds:
    // that path is joined, and a join that does not settle within the
    // bound leaves the claim exactly as it was, with no second command.
    registry.set_phase("A", Phase::Stopping(TerminationCause::SelectedTermination));
    *registry.time_out_waits.lock().unwrap() = true;
    let sent_before = routing.calls().len();
    let error = use_case.execute(request("A", 1, 1)).await.unwrap_err();
    assert!(
        matches!(
            error,
            TerminateDelegatedAgentError::TerminationFailed { .. }
        ),
        "{error:?}"
    );
    assert_eq!(routing.calls().len(), sent_before, "no second shutdown");
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

mod owner {
    //! The receiver as the direct owner of the target (#1936 review): the
    //! protocol, the observed exit, the owned-handle fallback and the
    //! compensation all run here, and only a failure after effects were
    //! dispatched keeps the claim.
    use std::sync::Arc;

    use super::super::super::lifecycle_fakes::{
        FakeCompensation, FakeRegistry, FakeTermination, Phase,
    };
    use super::super::super::teardown_fakes::{FakeLifecycle, FakeRouting, root_tree};
    use super::*;
    use crate::application::subagents::dto::TerminationResult;
    use crate::application::subagents::ports::{
        DownstreamRejection, TerminationCause, TerminationConclusion,
    };

    struct Rig {
        registry: Arc<FakeRegistry>,
        routing: Arc<FakeRouting>,
        termination: Arc<FakeTermination>,
        compensation: Arc<FakeCompensation>,
        use_case: TerminateDelegatedAgent,
    }

    fn rig(registry: Arc<FakeRegistry>, conclusion: TerminationConclusion) -> Rig {
        let lifecycle = FakeLifecycle::new(root_tree());
        let routing = FakeRouting::new();
        let termination = FakeTermination::new(registry.clone(), conclusion);
        let compensation = FakeCompensation::new(registry.clone());
        let use_case = TerminateDelegatedAgent::new(lifecycle, routing.clone())
            .with_owner_conclusion(OwnerConclusionPorts {
                registry: registry.clone(),
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

    fn owned(uuid: &str) -> Arc<FakeRegistry> {
        FakeRegistry::new()
            .with_row(uuid, 1, uuid)
            .holding_process(uuid)
    }

    #[tokio::test]
    async fn an_acknowledged_child_that_exits_is_concluded_and_compensated_here() {
        let rig = rig(owned("A"), TerminationConclusion::ExitedAfterProtocol);
        let routed = rig.use_case.execute(request("A", 1, 1)).await.unwrap();
        assert_eq!(
            routed,
            TerminationRouted::ShutdownRequested {
                child: identity("A", 1),
                result: Some(TerminationResult::Graceful),
            }
        );
        assert_eq!(rig.termination.calls().len(), 1);
        assert_eq!(
            rig.compensation.calls(),
            [(identity("A", 1), TerminationCause::SelectedTermination)]
        );
        assert_eq!(
            rig.registry.trace(),
            ["claim-stopping A", "claim-terminal A", "compensated A"]
        );
    }

    #[tokio::test]
    async fn an_acknowledged_child_that_never_exits_is_ended_by_the_owned_handle() {
        let rig = rig(owned("A"), TerminationConclusion::ExitedAfterFallback);
        let routed = rig.use_case.execute(request("A", 1, 1)).await.unwrap();
        assert!(matches!(
            routed,
            TerminationRouted::ShutdownRequested {
                result: Some(TerminationResult::Fallback),
                ..
            }
        ));
        assert_eq!(rig.registry.phase("A"), Phase::Compensated);
    }

    #[tokio::test]
    async fn a_fallback_that_could_not_end_the_child_keeps_the_claim() {
        let rig = rig(
            owned("A"),
            TerminationConclusion::StillRunning("kill sent".into()),
        );
        let error = rig.use_case.execute(request("A", 1, 1)).await.unwrap_err();
        assert_eq!(
            error,
            TerminateDelegatedAgentError::TerminationFailed {
                child: AgentUuid::new("A"),
                detail: "kill sent".into(),
            }
        );
        assert_eq!(
            rig.registry.phase("A"),
            Phase::StoppingReturned(TerminationCause::SelectedTermination),
            "effects were dispatched: the eventual exit is this kill's, and a retry may re-take it"
        );
        assert!(rig.compensation.calls().is_empty());
    }

    /// #1953 review (4): a kill of the same target another path is still
    /// executing is joined — no second `shutdown`, no parallel conclusion —
    /// and answered once that path's compensation ran; the joined path's
    /// result is not this hop's to report.
    #[tokio::test]
    async fn a_target_another_owner_is_still_ending_is_joined_not_shut_down_twice() {
        let rig = rig(owned("A"), TerminationConclusion::ExitedAfterProtocol);
        rig.registry
            .set_phase("A", Phase::Stopping(TerminationCause::SelectedTermination));
        let registry = rig.registry.clone();
        let joiner = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            registry.set_phase("A", Phase::Compensated);
        });
        let routed = rig.use_case.execute(request("A", 1, 1)).await.unwrap();
        joiner.await.unwrap();
        assert_eq!(
            routed,
            TerminationRouted::ShutdownRequested {
                child: identity("A", 1),
                result: None,
            }
        );
        assert!(rig.routing.calls().is_empty(), "no second shutdown");
        assert!(rig.termination.calls().is_empty(), "no parallel conclusion");
        assert!(rig.compensation.calls().is_empty());
        assert_eq!(rig.registry.trace(), ["await-compensated A"]);
    }

    /// #1953 review (1): a claim whose owner returned after effects (a
    /// kill that failed) is re-taken by the next kill, which re-attempts
    /// the protocol — a second `shutdown` — instead of being refused or
    /// joining a path that is no longer executing.
    #[tokio::test]
    async fn a_claim_whose_owner_returned_is_re_taken_and_the_protocol_re_attempted() {
        let rig = rig(owned("A"), TerminationConclusion::ExitedAfterProtocol);
        rig.registry.set_phase(
            "A",
            Phase::StoppingReturned(TerminationCause::SelectedTermination),
        );
        let routed = rig.use_case.execute(request("A", 1, 1)).await.unwrap();
        assert!(matches!(
            routed,
            TerminationRouted::ShutdownRequested {
                result: Some(TerminationResult::Graceful),
                ..
            }
        ));
        assert_eq!(
            rig.routing.calls().len(),
            1,
            "the protocol was re-attempted"
        );
        assert_eq!(
            rig.registry.trace(),
            ["re-take-stopping A", "claim-terminal A", "compensated A"]
        );
    }

    /// #1953 review (8): a child that answers `shutdown` with an
    /// `already_exited` refusal is an acknowledged attempt here too: the
    /// owned handle waits for the exit, no signal is authorised by the
    /// refusal, and the conclusion is reported as observed.
    #[tokio::test]
    async fn a_child_already_ending_is_awaited_as_an_acknowledgement_by_its_owner() {
        let rig = rig(owned("A"), TerminationConclusion::ExitedAfterProtocol);
        rig.routing
            .downstream
            .lock()
            .unwrap()
            .push((AgentUuid::new("A"), DownstreamRejection::AlreadyExited));
        let routed = rig.use_case.execute(request("A", 1, 1)).await.unwrap();
        assert!(matches!(
            routed,
            TerminationRouted::ShutdownRequested {
                result: Some(TerminationResult::Graceful),
                ..
            }
        ));
        let attempts: Vec<_> = rig
            .termination
            .calls()
            .into_iter()
            .map(|(_, attempt, _)| attempt)
            .collect();
        assert_eq!(
            attempts,
            [crate::application::subagents::ports::ProtocolAttempt::Acknowledged]
        );
    }

    #[tokio::test]
    async fn an_unreachable_child_without_a_handle_lifts_the_claim() {
        let rig = rig(
            FakeRegistry::new().with_row("A", 1, "alpha"),
            TerminationConclusion::NoRetainedHandle,
        );
        rig.routing
            .unreachable
            .lock()
            .unwrap()
            .push(AgentUuid::new("A"));
        let error = rig.use_case.execute(request("A", 1, 1)).await.unwrap_err();
        assert!(matches!(
            error,
            TerminateDelegatedAgentError::ChildUnreachable { .. }
        ));
        assert_eq!(rig.registry.phase("A"), Phase::Live);
        assert_eq!(
            rig.registry.trace(),
            ["claim-stopping A", "release-stopping A"]
        );
    }

    #[tokio::test]
    async fn a_child_whose_reaper_won_during_the_protocol_is_already_exited() {
        let rig = rig(
            FakeRegistry::new().with_row("A", 1, "alpha"),
            TerminationConclusion::NoRetainedHandle,
        );
        rig.routing
            .unreachable
            .lock()
            .unwrap()
            .push(AgentUuid::new("A"));
        // The reaper claims the terminal effects while the edge is in
        // flight: the row is already compensated when the answer comes.
        *rig.routing.hold_child.lock().unwrap() = Some(AgentUuid::new("A"));
        let registry = rig.registry.clone();
        let routing = rig.routing.clone();
        let reaper = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            registry.set_phase("A", Phase::Compensated);
            routing.gate.notify_one();
        });
        let routed = rig.use_case.execute(request("A", 1, 1)).await.unwrap();
        reaper.await.unwrap();
        assert!(matches!(
            routed,
            TerminationRouted::ShutdownRequested {
                result: Some(TerminationResult::AlreadyExited),
                ..
            }
        ));
        assert!(rig.compensation.calls().is_empty(), "joined, not repeated");
        // A row already terminal before the edge is answered without one.
        let terminal = self::rig(
            FakeRegistry::new().with_row("A", 1, "alpha"),
            TerminationConclusion::NoRetainedHandle,
        );
        terminal.registry.set_phase("A", Phase::Compensated);
        let error = terminal
            .use_case
            .execute(request("A", 1, 1))
            .await
            .unwrap_err();
        assert_eq!(
            error,
            TerminateDelegatedAgentError::TargetAlreadyExited(AgentUuid::new("A"))
        );
        assert!(terminal.routing.calls().is_empty());
    }

    #[tokio::test]
    async fn an_acknowledged_member_without_a_handle_whose_exit_is_not_observed_keeps_the_claim() {
        let registry = FakeRegistry::new().with_row("A", 1, "alpha");
        *registry.time_out_waits.lock().unwrap() = true;
        let rig = rig(registry, TerminationConclusion::NoRetainedHandle);
        let error = rig.use_case.execute(request("A", 1, 1)).await.unwrap_err();
        assert!(matches!(
            error,
            TerminateDelegatedAgentError::TerminationFailed { .. }
        ));
        assert_eq!(
            rig.registry.phase("A"),
            Phase::StoppingReturned(TerminationCause::SelectedTermination)
        );
    }

    #[tokio::test]
    async fn a_target_this_harness_already_saw_exit_is_reported_as_such() {
        // The lineage no longer lists Q (it is exited) but the registry
        // retains its compensated row.
        let registry = FakeRegistry::new().with_row("Q", 1, "quebec");
        registry.set_phase("Q", Phase::Compensated);
        let rig = rig(registry, TerminationConclusion::NoRetainedHandle);
        let error = rig.use_case.execute(request("Q", 1, 1)).await.unwrap_err();
        assert_eq!(
            error,
            TerminateDelegatedAgentError::TargetAlreadyExited(AgentUuid::new("Q"))
        );
        assert!(rig.routing.calls().is_empty());
    }

    /// An intermediate lifts its own claim on every downstream answer —
    /// a relayed result, a refusal, a timeout — and relays the answer.
    #[tokio::test]
    async fn an_intermediate_relays_the_downstream_answer_and_lifts_its_claim() {
        let rig = rig(
            FakeRegistry::new()
                .with_row("A", 1, "alpha")
                .with_row("B", 1, "bravo"),
            TerminationConclusion::NoRetainedHandle,
        );
        *rig.routing.forward_result.lock().unwrap() = Some(TerminationResult::Fallback);
        let routed = rig.use_case.execute(request("B", 1, 2)).await.unwrap();
        assert!(matches!(
            routed,
            TerminationRouted::Forwarded {
                result: Some(TerminationResult::Fallback),
                ..
            }
        ));
        assert_eq!(rig.registry.phase("B"), Phase::Live);
        assert_eq!(
            rig.registry.trace(),
            ["claim-stopping B", "release-stopping B"]
        );
        for rejection in [
            DownstreamRejection::Failed("no exit".into()),
            DownstreamRejection::AlreadyExited,
            DownstreamRejection::UnknownTarget,
        ] {
            *rig.routing.downstream.lock().unwrap() =
                vec![(AgentUuid::new("A"), rejection.clone())];
            let error = rig.use_case.execute(request("B", 1, 2)).await.unwrap_err();
            assert_eq!(
                error,
                TerminateDelegatedAgentError::Downstream {
                    via: AgentUuid::new("A"),
                    rejection,
                }
            );
            assert_eq!(rig.registry.phase("B"), Phase::Live, "released");
        }
        rig.routing.downstream.lock().unwrap().clear();
        rig.routing
            .unreachable
            .lock()
            .unwrap()
            .push(AgentUuid::new("A"));
        let error = rig.use_case.execute(request("B", 1, 2)).await.unwrap_err();
        assert!(matches!(
            error,
            TerminateDelegatedAgentError::ChildUnreachable { child, .. } if child == AgentUuid::new("A")
        ));
        assert_eq!(rig.registry.phase("B"), Phase::Live, "released on timeout");
        assert!(
            rig.termination.calls().is_empty(),
            "never a fallback for a nested target"
        );
    }

    #[test]
    fn new_errors_render_their_vocabulary() {
        assert_eq!(
            TerminateDelegatedAgentError::TargetAlreadyExited(AgentUuid::new("Q")).to_string(),
            "target Q already exited"
        );
        assert_eq!(
            TerminateDelegatedAgentError::TerminationFailed {
                child: AgentUuid::new("A"),
                detail: "x".into()
            }
            .to_string(),
            "termination of A failed: x"
        );
        assert_eq!(
            TerminateDelegatedAgentError::Downstream {
                via: AgentUuid::new("A"),
                rejection: DownstreamRejection::NotAccepting
            }
            .to_string(),
            "via A: downstream harness is not accepting"
        );
        for rejection in [
            DownstreamRejection::UnknownTarget,
            DownstreamRejection::StaleGeneration,
            DownstreamRejection::AlreadyExited,
            DownstreamRejection::Rejected("r".into()),
            DownstreamRejection::NotAccepting,
            DownstreamRejection::Unreachable("u".into()),
            DownstreamRejection::Failed("f".into()),
        ] {
            let detail = rejection.to_string();
            assert_eq!(
                DownstreamRejection::from_kind(rejection.kind(), &detail).kind(),
                rejection.kind()
            );
            assert_eq!(
                rejection.effects_dispatched(),
                matches!(rejection, DownstreamRejection::Failed(_))
            );
        }
        assert_eq!(
            DownstreamRejection::from_kind("mystery", "d"),
            DownstreamRejection::Rejected("d".into())
        );
    }
}
