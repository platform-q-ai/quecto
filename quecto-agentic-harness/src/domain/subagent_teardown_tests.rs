use super::*;

fn id(uuid: &str, generation: u64) -> DelegatedAgentIdentity {
    DelegatedAgentIdentity::new(uuid, LaunchGeneration::new(generation))
}

fn record(uuid: &str, generation: u64, parent: &str) -> LineageRecord {
    LineageRecord {
        identity: id(uuid, generation),
        parent: AgentUuid::new(parent),
    }
}

/// root → A → B, plus A's unrelated child C and root's unrelated child D.
fn tree() -> LineageSnapshot {
    LineageSnapshot {
        owner: AgentUuid::new("root"),
        records: vec![
            record("A", 1, "root"),
            record("B", 1, "A"),
            record("C", 3, "A"),
            record("D", 1, "root"),
        ],
    }
}

fn depth(hops: u32) -> RoutingDepth {
    RoutingDepth::new(hops).unwrap()
}

#[test]
fn routing_depth_rejects_zero_and_excess_and_accepts_bounds() {
    assert_eq!(RoutingDepth::new(0), Err(RoutingDepthError::Zero));
    assert_eq!(
        RoutingDepth::new(RoutingDepth::MAX_HOPS + 1),
        Err(RoutingDepthError::ExceedsMaximum {
            requested: 33,
            maximum: 32
        })
    );
    assert_eq!(depth(1).hops(), 1);
    assert_eq!(depth(RoutingDepth::MAX_HOPS).hops(), 32);
    assert_eq!(depth(1).after_forward(), None);
    assert_eq!(depth(2).after_forward(), Some(depth(1)));
    assert_eq!(
        RoutingDepthError::Zero.to_string(),
        "remaining_depth must be at least 1"
    );
    assert_eq!(
        RoutingDepth::new(99).unwrap_err().to_string(),
        "remaining_depth 99 exceeds maximum 32"
    );
}

#[test]
fn shutdown_reason_round_trips_only_canonical_spellings() {
    for reason in ShutdownReason::ALL {
        assert_eq!(ShutdownReason::parse(reason.as_str()), Ok(reason));
        assert_eq!(reason.to_string(), reason.as_str());
    }
    for raw in [
        "",
        "Parent_Shutdown",
        "parent-shutdown",
        "kill",
        " parent_shutdown",
    ] {
        assert_eq!(
            ShutdownReason::parse(raw),
            Err(UnknownShutdownReason(raw.to_owned())),
            "{raw:?}"
        );
    }
    assert_eq!(
        UnknownShutdownReason("kill".into()).to_string(),
        "unknown shutdown reason \"kill\""
    );
}

#[test]
fn lifecycle_transitions_fence_work_before_termination() {
    use HarnessLifecycleState::*;
    assert!(Accepting.accepts_new_work());
    assert!(!Frozen.accepts_new_work());
    assert!(!Terminated.accepts_new_work());
    assert_eq!(Accepting.freeze(), Ok(Frozen));
    assert_eq!(Frozen.freeze(), Ok(Frozen));
    assert_eq!(Frozen.thaw(), Ok(Accepting));
    assert_eq!(Accepting.thaw(), Ok(Accepting));
    assert_eq!(Frozen.terminate(), Ok(Terminated));
    assert_eq!(Terminated.terminate(), Ok(Terminated));
    let err = Accepting.terminate().unwrap_err();
    assert_eq!(err.from, Accepting);
    assert_eq!(err.to_string(), "cannot terminate from Accepting");
    assert_eq!(
        Terminated.freeze().unwrap_err().to_string(),
        "cannot freeze from Terminated"
    );
    assert_eq!(
        Terminated.thaw().unwrap_err().to_string(),
        "cannot thaw from Terminated"
    );
}

#[test]
fn direct_children_are_records_parented_by_the_owner() {
    let snapshot = tree();
    let direct: Vec<_> = snapshot
        .direct_children()
        .map(|identity| identity.uuid.as_str().to_owned())
        .collect();
    assert_eq!(direct, ["A", "D"]);
}

#[test]
fn targeting_a_direct_child_resolves_to_self_shutdown_of_that_child() {
    assert_eq!(
        resolve_termination_route(&tree(), &id("A", 1), depth(1)),
        Ok(TerminationRoute::ShutdownDirectChild(id("A", 1)))
    );
    // Depth beyond what is needed is harmless for a direct child.
    assert_eq!(
        resolve_termination_route(&tree(), &id("A", 1), depth(32)),
        Ok(TerminationRoute::ShutdownDirectChild(id("A", 1)))
    );
}

#[test]
fn targeting_a_grandchild_forwards_via_the_intermediate_with_one_hop_consumed() {
    assert_eq!(
        resolve_termination_route(&tree(), &id("B", 1), depth(2)),
        Ok(TerminationRoute::ForwardToDirectChild {
            via: id("A", 1),
            remaining_depth: depth(1),
        })
    );
    assert_eq!(
        resolve_termination_route(&tree(), &id("B", 1), depth(5)),
        Ok(TerminationRoute::ForwardToDirectChild {
            via: id("A", 1),
            remaining_depth: depth(4),
        })
    );
}

#[test]
fn over_depth_route_is_refused_at_the_first_hop() {
    assert_eq!(
        resolve_termination_route(&tree(), &id("B", 1), depth(1)),
        Err(TerminationRouteError::DepthExhausted {
            target: AgentUuid::new("B"),
            remaining: depth(1),
        })
    );
    let deep = LineageSnapshot {
        owner: AgentUuid::new("root"),
        records: vec![
            record("A", 1, "root"),
            record("B", 1, "A"),
            record("C", 1, "B"),
            record("E", 1, "C"),
        ],
    };
    // E is four edges away: depth 3 cannot reach it even after forwarding.
    assert!(matches!(
        resolve_termination_route(&deep, &id("E", 1), depth(3)),
        Err(TerminationRouteError::DepthExhausted { .. })
    ));
    assert!(matches!(
        resolve_termination_route(&deep, &id("E", 1), depth(4)),
        Ok(TerminationRoute::ForwardToDirectChild { .. })
    ));
}

#[test]
fn stale_generation_is_rejected_even_for_a_known_uuid() {
    assert_eq!(
        resolve_termination_route(&tree(), &id("C", 2), depth(3)),
        Err(TerminationRouteError::StaleGeneration {
            target: AgentUuid::new("C"),
            requested: LaunchGeneration::new(2),
            current: LaunchGeneration::new(3),
        })
    );
}

#[test]
fn unknown_target_and_self_target_are_rejected() {
    assert_eq!(
        resolve_termination_route(&tree(), &id("zzz", 1), depth(3)),
        Err(TerminationRouteError::UnknownTarget(AgentUuid::new("zzz")))
    );
    assert_eq!(
        resolve_termination_route(&tree(), &id("root", 1), depth(3)),
        Err(TerminationRouteError::TargetIsSelf)
    );
}

#[test]
fn a_uuid_recorded_twice_is_ambiguous_and_yields_no_edge() {
    let doubled = LineageSnapshot {
        owner: AgentUuid::new("root"),
        records: vec![
            record("A", 1, "root"),
            record("B", 1, "A"),
            record("B", 1, "root"),
        ],
    };
    assert_eq!(
        resolve_termination_route(&doubled, &id("B", 1), depth(4)),
        Err(TerminationRouteError::AmbiguousLineage(AgentUuid::new("B")))
    );
    assert_eq!(
        TerminationRouteError::AmbiguousLineage(AgentUuid::new("B")).to_string(),
        "ambiguous lineage at B"
    );
}

#[test]
fn a_duplicated_intermediate_is_ambiguous_even_when_the_target_is_unique() {
    // A←root, D←root, B(1)←D, B(2)←A, C←B: targeting C must not pick D's B
    // just because it is listed first.
    let doubled = LineageSnapshot {
        owner: AgentUuid::new("root"),
        records: vec![
            record("A", 1, "root"),
            record("D", 1, "root"),
            record("B", 1, "D"),
            record("B", 2, "A"),
            record("C", 1, "B"),
        ],
    };
    assert_eq!(
        resolve_termination_route(&doubled, &id("C", 1), depth(8)),
        Err(TerminationRouteError::AmbiguousLineage(AgentUuid::new("B")))
    );
}

#[test]
fn a_three_edge_route_names_the_direct_child_at_every_hop() {
    // root → A → B → C, targeting C from each harness in turn.
    let root_view = LineageSnapshot {
        owner: AgentUuid::new("root"),
        records: vec![
            record("A", 1, "root"),
            record("B", 1, "A"),
            record("C", 1, "B"),
        ],
    };
    assert_eq!(
        resolve_termination_route(&root_view, &id("C", 1), depth(3)),
        Ok(TerminationRoute::ForwardToDirectChild {
            via: id("A", 1),
            remaining_depth: depth(2),
        })
    );
    let a_view = LineageSnapshot {
        owner: AgentUuid::new("A"),
        records: vec![record("B", 1, "A"), record("C", 1, "B")],
    };
    assert_eq!(
        resolve_termination_route(&a_view, &id("C", 1), depth(2)),
        Ok(TerminationRoute::ForwardToDirectChild {
            via: id("B", 1),
            remaining_depth: depth(1),
        })
    );
    let b_view = LineageSnapshot {
        owner: AgentUuid::new("B"),
        records: vec![record("C", 1, "B")],
    };
    assert_eq!(
        resolve_termination_route(&b_view, &id("C", 1), depth(1)),
        Ok(TerminationRoute::ShutdownDirectChild(id("C", 1)))
    );
}

#[test]
fn cyclic_or_dangling_lineage_never_yields_an_edge() {
    let cyclic = LineageSnapshot {
        owner: AgentUuid::new("root"),
        records: vec![record("X", 1, "Y"), record("Y", 1, "X")],
    };
    assert_eq!(
        resolve_termination_route(&cyclic, &id("X", 1), depth(8)),
        Err(TerminationRouteError::LineageCycle(AgentUuid::new("X")))
    );
    let dangling = LineageSnapshot {
        owner: AgentUuid::new("root"),
        records: vec![record("X", 1, "ghost")],
    };
    assert_eq!(
        resolve_termination_route(&dangling, &id("X", 1), depth(8)),
        Err(TerminationRouteError::LineageCycle(AgentUuid::new("ghost")))
    );
}

#[test]
fn route_errors_render_a_stable_vocabulary() {
    let messages = [
        TerminationRouteError::TargetIsSelf.to_string(),
        TerminationRouteError::UnknownTarget(AgentUuid::new("u")).to_string(),
        TerminationRouteError::AmbiguousLineage(AgentUuid::new("u")).to_string(),
        TerminationRouteError::StaleGeneration {
            target: AgentUuid::new("u"),
            requested: LaunchGeneration::new(1),
            current: LaunchGeneration::new(2),
        }
        .to_string(),
        TerminationRouteError::LineageCycle(AgentUuid::new("u")).to_string(),
        TerminationRouteError::DepthExhausted {
            target: AgentUuid::new("u"),
            remaining: depth(1),
        }
        .to_string(),
    ];
    assert_eq!(
        messages,
        [
            "target is the receiving harness; use shutdown",
            "unknown delegated agent u",
            "ambiguous lineage at u",
            "stale generation 1 for u (current 2)",
            "lineage cycle at u",
            "target u not reachable within 1 remaining hop(s)",
        ]
    );
}
