use std::sync::atomic::Ordering;

use super::rig::*;
use super::*;
use crate::application::subagents::dto::TerminationRouted;
use crate::application::subagents::ports::SubagentLifecycleRepository;
use crate::application::subagents::use_cases::teardown_fakes::*;
use crate::domain::ids::AgentUuid;
use crate::domain::subagent_teardown::{
    HarnessLifecycleState, LineageSnapshot, RoutingDepth, ShutdownReason,
};
use crate::interface::uds::subagent_teardown::ack_fakes::RecordingWriter;

#[tokio::test]
async fn a_three_edge_route_names_the_direct_child_at_every_hop() {
    // root → A → B → C; each harness in turn resolves exactly one edge.
    let views = [
        (
            LineageSnapshot {
                owner: AgentUuid::new("root"),
                records: vec![
                    record("A", 1, "root"),
                    record("B", 1, "A"),
                    record("C", 1, "B"),
                ],
            },
            3,
            Some("A"),
        ),
        (
            LineageSnapshot {
                owner: AgentUuid::new("A"),
                records: vec![record("B", 1, "A"), record("C", 1, "B")],
            },
            2,
            Some("B"),
        ),
        (
            LineageSnapshot {
                owner: AgentUuid::new("B"),
                records: vec![record("C", 1, "B")],
            },
            1,
            None,
        ),
    ];
    for (lineage, depth, via) in views {
        let rig = rig_for(lineage);
        let writer = RecordingWriter::default();
        let line = format!(
            r#"{{"type":"terminate_delegated_agent","id":"hop","target_uuid":"C","target_generation":1,"remaining_depth":{depth}}}"#
        );
        let outcome = rig
            .handle(
                &line,
                ConnectionAuthority::BoundParent,
                DeliveryState::Idle,
                &writer,
            )
            .await;
        let calls = rig.routing.calls();
        match via {
            Some(via) => {
                assert!(matches!(
                    &outcome,
                    ControllerOutcome::TerminationRouted {
                        outcome: Ok(TerminationRouted::Forwarded { via: v, remaining_depth }),
                        ..
                    } if v.uuid == AgentUuid::new(via) && remaining_depth.hops() == depth - 1
                ));
                assert_eq!(
                    calls,
                    [RoutingCall::Forward {
                        via: identity(via, 1),
                        target: identity("C", 1),
                        remaining_depth: RoutingDepth::new(depth - 1).unwrap(),
                    }]
                );
            }
            None => {
                assert!(matches!(
                    &outcome,
                    ControllerOutcome::TerminationRouted {
                        outcome: Ok(TerminationRouted::ShutdownRequested { child }),
                        ..
                    } if child.uuid == AgentUuid::new("C")
                ));
                assert_eq!(
                    calls,
                    [RoutingCall::Shutdown(
                        identity("C", 1),
                        ShutdownReason::SelectedTermination
                    )]
                );
            }
        }
        assert_eq!(rig.lifecycle.lifecycle(), HarnessLifecycleState::Accepting);
    }
}

#[tokio::test]
async fn unknown_reason_zero_depth_and_excess_depth_are_rejected_with_no_effect() {
    let rig = rig();
    let writer = RecordingWriter::default();
    let cases = [
        (
            r#"{"type":"shutdown","id":"r","reason":"kill"}"#,
            "unknown shutdown reason \"kill\"",
        ),
        (
            r#"{"type":"terminate_delegated_agent","id":"z","target_uuid":"A","target_generation":1,"remaining_depth":0}"#,
            "remaining_depth must be at least 1",
        ),
        (
            r#"{"type":"terminate_delegated_agent","id":"x","target_uuid":"A","target_generation":1,"remaining_depth":33}"#,
            "remaining_depth 33 exceeds maximum 32",
        ),
        (
            r#"{"type":"terminate_delegated_agent","id":"e","target_uuid":"","target_generation":1,"remaining_depth":1}"#,
            "target_uuid must not be empty",
        ),
    ];
    for (line, detail) in cases {
        let outcome = rig
            .handle(
                line,
                ConnectionAuthority::BoundParent,
                DeliveryState::Idle,
                &writer,
            )
            .await;
        assert!(
            matches!(&outcome, ControllerOutcome::Rejected { detail: d, written: Ok(()), .. } if d == detail),
            "{line}: {outcome:?}"
        );
    }
    rig.nothing_ran();
}

#[tokio::test]
async fn selected_routing_root_a_b_preserves_a_and_its_unrelated_children() {
    let rig = rig();
    let writer = RecordingWriter::default();
    let outcome = rig
        .handle(
            r#"{"type":"terminate_delegated_agent","id":"t-b","target_uuid":"B","target_generation":1,"remaining_depth":2}"#,
            ConnectionAuthority::BoundParent,
            DeliveryState::Busy,
            &writer,
        )
        .await;
    let ControllerOutcome::TerminationRouted {
        delivery: DeliveryState::Busy,
        outcome:
            Ok(TerminationRouted::Forwarded {
                via,
                remaining_depth,
            }),
        written: Ok(()),
    } = outcome
    else {
        panic!("expected forward, got {outcome:?}");
    };
    assert_eq!(via.uuid, AgentUuid::new("A"));
    assert_eq!(remaining_depth, RoutingDepth::new(1).unwrap());
    assert_eq!(
        rig.routing.calls(),
        [RoutingCall::Forward {
            via: identity("A", 1),
            target: identity("B", 1),
            remaining_depth: RoutingDepth::new(1).unwrap(),
        }]
    );
    let frame = frame_json(&writer.frames()[0]);
    assert_eq!(frame["id"], "t-b");
    assert_eq!(frame["data"]["status"], "forwarded");
    assert_eq!(frame["data"]["via_uuid"], "A");
    assert_eq!(frame["data"]["remaining_depth"], 1);
    // This harness stays alive and accepting; nothing else was touched.
    assert_eq!(rig.lifecycle.lifecycle(), HarnessLifecycleState::Accepting);
    assert_eq!(rig.cancellation.calls.load(Ordering::SeqCst), 0);
    assert!(rig.exit.signalled.lock().unwrap().is_empty());
}

#[tokio::test]
async fn selected_routing_targeting_a_direct_child_uses_self_shutdown_on_it() {
    let rig = rig();
    let writer = RecordingWriter::default();
    let outcome = rig
        .handle(
            r#"{"type":"terminate_delegated_agent","id":"t-a","target_uuid":"A","target_generation":1,"remaining_depth":1}"#,
            ConnectionAuthority::LocalOperator,
            DeliveryState::Idle,
            &writer,
        )
        .await;
    assert!(matches!(
        outcome,
        ControllerOutcome::TerminationRouted {
            outcome: Ok(TerminationRouted::ShutdownRequested { .. }),
            written: Ok(()),
            ..
        }
    ));
    assert_eq!(
        rig.routing.calls(),
        [RoutingCall::Shutdown(
            identity("A", 1),
            ShutdownReason::SelectedTermination
        )]
    );
    let frame = frame_json(&writer.frames()[0]);
    assert_eq!(frame["data"]["status"], "shutdown_requested");
    assert_eq!(frame["data"]["child_uuid"], "A");
    assert_eq!(rig.lifecycle.lifecycle(), HarnessLifecycleState::Accepting);
}

#[tokio::test]
async fn stale_generation_cycle_and_over_depth_routes_are_rejected_with_no_effect() {
    let cyclic = LineageSnapshot {
        owner: AgentUuid::new("root"),
        records: vec![record("X", 1, "Y"), record("Y", 1, "X")],
    };
    let cases = [
        (
            root_tree(),
            r#"{"type":"terminate_delegated_agent","id":"s","target_uuid":"A","target_generation":9,"remaining_depth":1}"#,
            "termination rejected: stale generation 9 for A (current 1)",
        ),
        (
            root_tree(),
            r#"{"type":"terminate_delegated_agent","id":"d","target_uuid":"B","target_generation":1,"remaining_depth":1}"#,
            "termination rejected: target B not reachable within 1 remaining hop(s)",
        ),
        (
            cyclic,
            r#"{"type":"terminate_delegated_agent","id":"c","target_uuid":"X","target_generation":1,"remaining_depth":4}"#,
            "termination rejected: lineage cycle at X",
        ),
        (
            root_tree(),
            r#"{"type":"terminate_delegated_agent","id":"u","target_uuid":"root","target_generation":1,"remaining_depth":4}"#,
            "termination rejected: target is the receiving harness; use shutdown",
        ),
    ];
    for (lineage, line, expected) in cases {
        let rig = rig_for(lineage);
        let writer = RecordingWriter::default();
        let outcome = rig
            .handle(
                line,
                ConnectionAuthority::BoundParent,
                DeliveryState::Idle,
                &writer,
            )
            .await;
        let ControllerOutcome::TerminationRouted {
            outcome: Err(error),
            written: Ok(()),
            ..
        } = outcome
        else {
            panic!("expected rejection for {line}: {outcome:?}");
        };
        assert_eq!(error.to_string(), expected);
        let frame = frame_json(&writer.frames()[0]);
        assert_eq!(frame["success"], false);
        assert_eq!(frame["error"], expected);
        rig.nothing_ran();
    }
}
