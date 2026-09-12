use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::*;
use crate::application::subagents::dto::TerminationRouted;
use crate::application::subagents::ports::SubagentLifecycleRepository;
use crate::application::subagents::use_cases::teardown_fakes::*;
use crate::application::subagents::use_cases::{
    ExecuteHarnessShutdownPorts, HarnessShutdownTransaction,
};
use crate::domain::ids::AgentUuid;
use crate::domain::subagent_teardown::{
    HarnessLifecycleState, LineageSnapshot, RoutingDepth, ShutdownReason,
};
use crate::interface::uds::subagent_teardown::ack_fakes::RecordingWriter;
use crate::interface::uds::subagent_teardown::wire::TEARDOWN_COMMAND_CAP_BYTES;

struct Rig {
    lifecycle: Arc<FakeLifecycle>,
    routing: Arc<FakeRouting>,
    cancellation: Arc<FakeCancellation>,
    persistence: Arc<FakePersistence>,
    exit: Arc<FakeExit>,
    controller: SubagentTeardownController,
}

fn rig_for(lineage: LineageSnapshot) -> Rig {
    let lifecycle = FakeLifecycle::new(lineage);
    let routing = FakeRouting::new();
    let cancellation = FakeCancellation::new(true);
    let persistence = FakePersistence::new();
    let exit = FakeExit::new();
    let transaction = HarnessShutdownTransaction::new(lifecycle.clone(), FakeClock::at(42));
    let prepare = Arc::new(PrepareHarnessShutdown::new(transaction.clone()));
    let execute = Arc::new(ExecuteHarnessShutdown::new(
        transaction,
        ExecuteHarnessShutdownPorts {
            routing: routing.clone(),
            cancellation: cancellation.clone(),
            persistence: persistence.clone(),
            exit: exit.clone(),
        },
    ));
    let terminate = Arc::new(TerminateDelegatedAgent::new(
        lifecycle.clone(),
        routing.clone(),
    ));
    Rig {
        lifecycle,
        routing,
        cancellation,
        persistence,
        exit,
        controller: SubagentTeardownController::new(prepare, execute, terminate),
    }
}

fn rig() -> Rig {
    rig_for(root_tree())
}

impl Rig {
    fn nothing_ran(&self) {
        assert!(self.routing.calls().is_empty());
        assert_eq!(self.cancellation.calls.load(Ordering::SeqCst), 0);
        assert!(self.persistence.calls.lock().unwrap().is_empty());
        assert!(self.exit.signalled.lock().unwrap().is_empty());
        assert_eq!(self.lifecycle.lifecycle(), HarnessLifecycleState::Accepting);
    }

    async fn handle(
        &self,
        line: &str,
        authority: ConnectionAuthority,
        delivery: DeliveryState,
        writer: &RecordingWriter,
    ) -> ControllerOutcome {
        self.controller
            .handle(line, authority, delivery, writer)
            .await
    }
}

const SHUTDOWN_LINE: &str = r#"{"type":"shutdown","id":"c-1","reason":"parent_shutdown"}"#;

fn frame_json(frame: &str) -> serde_json::Value {
    serde_json::from_str(frame.trim_end()).unwrap()
}

#[tokio::test]
async fn shutdown_flushes_the_correlated_ack_before_the_token_reaches_execute() {
    for delivery in [DeliveryState::Idle, DeliveryState::Busy] {
        let rig = rig();
        let writer = RecordingWriter::default();
        let outcome = rig
            .handle(
                SHUTDOWN_LINE,
                ConnectionAuthority::BoundParent,
                delivery,
                &writer,
            )
            .await;
        let ControllerOutcome::ShutdownExecuted {
            delivery: recorded,
            outcome,
        } = outcome
        else {
            panic!("expected executed shutdown, got {outcome:?}");
        };
        assert_eq!(recorded, delivery);
        let outcome = outcome.unwrap();
        assert_eq!(outcome.reason, ShutdownReason::ParentShutdown);
        assert!(outcome.turn_cancelled);
        let frames = writer.frames();
        assert_eq!(frames.len(), 1, "exactly one ACK frame");
        let ack = frame_json(&frames[0]);
        assert_eq!(ack["id"], "c-1");
        assert_eq!(ack["command"], "shutdown");
        assert_eq!(ack["success"], true);
        assert_eq!(ack["data"]["status"], "shutting_down");
        assert_eq!(ack["data"]["reason"], "parent_shutdown");
        // The ACK was flushed strictly before any teardown effect: the writer
        // trace has the flush and the cancellation counter only moved after.
        assert_eq!(*writer.trace.lock().unwrap(), ["ack-flushed"]);
        assert_eq!(rig.cancellation.calls.load(Ordering::SeqCst), 1);
        assert_eq!(rig.routing.calls().len(), 2);
        assert_eq!(rig.lifecycle.lifecycle(), HarnessLifecycleState::Terminated);
    }
}

#[tokio::test]
async fn ack_flush_failure_releases_the_admission_and_never_executes() {
    let rig = rig();
    let writer = RecordingWriter::failing();
    let outcome = rig
        .handle(
            SHUTDOWN_LINE,
            ConnectionAuthority::LocalOperator,
            DeliveryState::Busy,
            &writer,
        )
        .await;
    assert_eq!(
        outcome,
        ControllerOutcome::ShutdownAbandoned {
            ack_error: AckWriteError("broken pipe".into()),
            release: Ok(ReleaseOutcome::Released),
        }
    );
    assert_eq!(writer.attempts.load(Ordering::SeqCst), 1);
    assert!(writer.frames().is_empty());
    rig.nothing_ran();
    // The harness is usable again: a later command on a working connection
    // admits afresh and executes.
    let working = RecordingWriter::default();
    let outcome = rig
        .handle(
            SHUTDOWN_LINE,
            ConnectionAuthority::BoundParent,
            DeliveryState::Idle,
            &working,
        )
        .await;
    assert!(matches!(
        outcome,
        ControllerOutcome::ShutdownExecuted { outcome: Ok(_), .. }
    ));
}

#[tokio::test]
async fn unauthenticated_connections_are_rejected_with_a_correlated_error_and_no_effect() {
    let rig = rig();
    let writer = RecordingWriter::default();
    let outcome = rig
        .handle(
            SHUTDOWN_LINE,
            ConnectionAuthority::Unauthenticated,
            DeliveryState::Idle,
            &writer,
        )
        .await;
    assert_eq!(
        outcome,
        ControllerOutcome::Rejected {
            command: "shutdown",
            detail: "unauthorized connection".into(),
            written: Ok(()),
        }
    );
    let frame = frame_json(&writer.frames()[0]);
    assert_eq!(frame["id"], "c-1");
    assert_eq!(frame["success"], false);
    assert_eq!(frame["error"], "unauthorized connection");
    rig.nothing_ran();
    let terminate = r#"{"type":"terminate_delegated_agent","id":"t","target_uuid":"A","target_generation":1,"remaining_depth":1}"#;
    let outcome = rig
        .handle(
            terminate,
            ConnectionAuthority::Unauthenticated,
            DeliveryState::Idle,
            &writer,
        )
        .await;
    assert!(matches!(
        outcome,
        ControllerOutcome::Rejected {
            command: "terminate_delegated_agent",
            ..
        }
    ));
    rig.nothing_ran();
}

#[tokio::test]
async fn framing_and_shape_violations_are_rejected_before_authorization_matters() {
    let rig = rig();
    let writer = RecordingWriter::default();
    let oversized = format!(
        r#"{{"type":"shutdown","reason":"{}"}}"#,
        "x".repeat(TEARDOWN_COMMAND_CAP_BYTES)
    );
    let malformed = r#"{"type":"shutdown","reason":"parent_shutdown","ack":"accept"}"#;
    for line in [oversized.as_str(), malformed, "garbage"] {
        let outcome = rig
            .handle(
                line,
                ConnectionAuthority::BoundParent,
                DeliveryState::Idle,
                &writer,
            )
            .await;
        let ControllerOutcome::Rejected {
            command, written, ..
        } = outcome
        else {
            panic!("expected rejection for {line}");
        };
        assert_eq!(command, UNPARSED_COMMAND);
        assert_eq!(written, Ok(()));
    }
    assert_eq!(writer.frames().len(), 3);
    rig.nothing_ran();
    // A shape-invalid command that carried an id gets a correlated rejection.
    let correlated = RecordingWriter::default();
    rig.handle(
        r#"{"type":"shutdown","id":"bad-1","reason":["x"]}"#,
        ConnectionAuthority::BoundParent,
        DeliveryState::Idle,
        &correlated,
    )
    .await;
    let frame = frame_json(&correlated.frames()[0]);
    assert_eq!(frame["id"], "bad-1");
    assert_eq!(frame["success"], false);
    rig.nothing_ran();
}

#[tokio::test]
async fn non_teardown_lines_are_ignored_without_a_frame() {
    let rig = rig();
    let writer = RecordingWriter::default();
    let outcome = rig
        .handle(
            r#"{"type":"prompt","message":"hi"}"#,
            ConnectionAuthority::BoundParent,
            DeliveryState::Idle,
            &writer,
        )
        .await;
    assert_eq!(outcome, ControllerOutcome::Ignored);
    assert!(writer.frames().is_empty());
    rig.nothing_ran();
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
            r#"{"type":"terminate_delegated_agent","id":"x","target_uuid":"A","target_generation":1,"remaining_depth":17}"#,
            "remaining_depth 17 exceeds maximum 16",
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
async fn duplicate_shutdown_commands_join_one_outcome_and_each_gets_its_own_ack() {
    let rig = rig();
    let first_writer = RecordingWriter::default();
    let first = rig
        .handle(
            SHUTDOWN_LINE,
            ConnectionAuthority::BoundParent,
            DeliveryState::Idle,
            &first_writer,
        )
        .await;
    let second_writer = RecordingWriter::default();
    let second = rig
        .handle(
            r#"{"type":"shutdown","id":"c-2","reason":"operator_request"}"#,
            ConnectionAuthority::LocalOperator,
            DeliveryState::Idle,
            &second_writer,
        )
        .await;
    let (
        ControllerOutcome::ShutdownExecuted { outcome: a, .. },
        ControllerOutcome::ShutdownExecuted { outcome: b, .. },
    ) = (first, second)
    else {
        panic!("both commands execute");
    };
    assert_eq!(a, b);
    assert_eq!(frame_json(&second_writer.frames()[0])["id"], "c-2");
    // The joiner's ACK still reports the admitted reason.
    assert_eq!(
        frame_json(&second_writer.frames()[0])["data"]["reason"],
        "parent_shutdown"
    );
    assert_eq!(rig.cancellation.calls.load(Ordering::SeqCst), 1);
    assert_eq!(rig.routing.calls().len(), 2);
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

#[tokio::test]
async fn prepare_rejection_on_a_terminated_harness_is_presented_not_executed() {
    let rig = rig();
    rig.lifecycle.force(HarnessLifecycleState::Terminated);
    let writer = RecordingWriter::default();
    let outcome = rig
        .handle(
            SHUTDOWN_LINE,
            ConnectionAuthority::BoundParent,
            DeliveryState::Idle,
            &writer,
        )
        .await;
    assert_eq!(
        outcome,
        ControllerOutcome::Rejected {
            command: "shutdown",
            detail: "harness already terminated".into(),
            written: Ok(()),
        }
    );
    assert_eq!(rig.cancellation.calls.load(Ordering::SeqCst), 0);
    assert!(rig.routing.calls().is_empty());
}

#[test]
fn only_bound_parent_and_local_operator_may_tear_down() {
    assert!(ConnectionAuthority::BoundParent.may_tear_down());
    assert!(ConnectionAuthority::LocalOperator.may_tear_down());
    assert!(!ConnectionAuthority::Unauthenticated.may_tear_down());
}
