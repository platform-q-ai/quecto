use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::rig::*;
use super::*;
use crate::application::subagents::ports::ExitReadiness;
use crate::application::subagents::ports::SubagentLifecycleRepository;
use crate::application::subagents::use_cases::teardown_fakes::*;
use crate::domain::subagent_teardown::{HarnessLifecycleState, ShutdownReason};
use crate::interface::uds::subagent_teardown::ack_fakes::RecordingWriter;
use crate::interface::uds::subagent_teardown::wire::TEARDOWN_COMMAND_CAP_BYTES;

#[tokio::test]
async fn shutdown_flushes_the_correlated_ack_before_the_token_reaches_execute() {
    for delivery in [DeliveryState::Idle, DeliveryState::Busy] {
        let rig = rig();
        let writer = traced_writer(&rig);
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
        // The ACK was flushed strictly before the first teardown effect: both
        // sides write the same trace, so a reversed order would show up.
        assert_eq!(
            *writer.trace.lock().unwrap(),
            ["ack-flushed", "execute-started"]
        );
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
async fn dropping_the_task_before_the_ack_flushes_releases_the_holder() {
    let rig = rig();
    let writer = Arc::new(RecordingWriter::stalled());
    let task = tokio::spawn({
        let controller = rig.controller.clone();
        let writer = writer.clone();
        async move {
            controller
                .handle(
                    SHUTDOWN_LINE,
                    ConnectionAuthority::BoundParent,
                    DeliveryState::Idle,
                    writer.as_ref(),
                )
                .await
        }
    });
    while writer.attempts.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    assert_eq!(rig.lifecycle.lifecycle(), HarnessLifecycleState::Frozen);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    // No ACK reached the peer, so nothing may run and nothing stays frozen.
    assert!(writer.frames().is_empty());
    rig.nothing_ran();
    assert_eq!(rig.lifecycle.lifecycle(), HarnessLifecycleState::Accepting);
}

#[tokio::test]
async fn dropping_the_task_after_the_ack_still_completes_the_shutdown() {
    let rig = rig_with(root_tree(), FakeCancellation::holding());
    let writer = Arc::new(RecordingWriter::default());
    let task = tokio::spawn({
        let controller = rig.controller.clone();
        let writer = writer.clone();
        async move {
            controller
                .handle(
                    SHUTDOWN_LINE,
                    ConnectionAuthority::BoundParent,
                    DeliveryState::Busy,
                    writer.as_ref(),
                )
                .await
        }
    });
    // Wait until the ACK is on the wire and the run is parked in cancellation.
    while rig.cancellation.calls.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    assert_eq!(writer.frames().len(), 1);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    // The peer saw an ACK: the shutdown must finish without the caller.
    rig.cancellation.gate.notify_one();
    rig.spawner.latest_finished().await;
    assert_eq!(rig.cancellation.calls.load(Ordering::SeqCst), 1);
    assert_eq!(rig.routing.calls().len(), 2);
    assert_eq!(rig.persistence.calls.lock().unwrap().len(), 1);
    assert_eq!(rig.exit.signalled.lock().unwrap().len(), 1);
    assert_eq!(rig.lifecycle.lifecycle(), HarnessLifecycleState::Terminated);
}

#[tokio::test]
async fn an_interrupted_run_is_re_driven_until_the_shutdown_completes() {
    let rig = rig();
    // The first two spawns are dropped by their runtime; the re-drive must
    // keep going until one runs.
    rig.spawner.drop_next.store(2, Ordering::SeqCst);
    let writer = RecordingWriter::default();
    let outcome = rig
        .handle(
            SHUTDOWN_LINE,
            ConnectionAuthority::BoundParent,
            DeliveryState::Idle,
            &writer,
        )
        .await;
    assert_eq!(writer.frames().len(), 1);
    assert!(matches!(
        outcome,
        ControllerOutcome::ShutdownExecuted { outcome: Ok(_), .. }
    ));
    assert_eq!(rig.spawner.spawned.load(Ordering::SeqCst), 3);
    assert_eq!(
        rig.exit.signalled(),
        [ExitReadiness::Completed(ShutdownReason::ParentShutdown)]
    );
    assert_eq!(rig.exit.signalled.lock().unwrap().len(), 1);
    assert_eq!(rig.lifecycle.lifecycle(), HarnessLifecycleState::Terminated);
}

#[tokio::test]
async fn re_driving_is_bounded_when_the_spawner_never_runs_anything() {
    let rig = rig();
    rig.spawner.drop_next.store(u64::MAX, Ordering::SeqCst);
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
        ControllerOutcome::ShutdownExecuted {
            delivery: DeliveryState::Idle,
            outcome: Err(HarnessShutdownError::ExecutionInterrupted),
        }
    );
    // First attempt plus MAX_REDRIVES re-drives.
    assert_eq!(
        rig.spawner.spawned.load(Ordering::SeqCst),
        u64::from(MAX_REDRIVES) + 1
    );
    // The parent was ACKed, so composition is told to exit anyway with the
    // failure recorded; the admission is intact for a later trigger.
    assert_eq!(
        rig.exit.signalled(),
        [ExitReadiness::Abandoned {
            reason: ShutdownReason::ParentShutdown,
            detail: "shutdown run interrupted 5 times after its ACK was flushed".into(),
        }]
    );
    assert_eq!(rig.lifecycle.lifecycle(), HarnessLifecycleState::Frozen);
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
    for line in [oversized.as_str(), malformed] {
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
    assert_eq!(writer.frames().len(), 2);
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
async fn unclaimed_lines_are_ignored_without_a_frame() {
    // Only the two teardown `type`s are claimed; everything else is left to
    // the ordinary dispatcher so it is never answered twice.
    let rig = rig();
    let writer = RecordingWriter::default();
    for line in [
        r#"{"type":"prompt","message":"hi"}"#,
        "garbage",
        "",
        "[]",
        r#"{"reason":"parent_shutdown"}"#,
    ] {
        let outcome = rig
            .handle(
                line,
                ConnectionAuthority::Unauthenticated,
                DeliveryState::Idle,
                &writer,
            )
            .await;
        assert_eq!(outcome, ControllerOutcome::Ignored, "{line}");
    }
    assert!(writer.frames().is_empty());
    rig.nothing_ran();
}

#[tokio::test]
async fn a_non_string_id_is_rejected_with_the_id_rendered_as_text() {
    let rig = rig();
    let writer = RecordingWriter::default();
    let outcome = rig
        .handle(
            r#"{"type":"shutdown","id":42,"reason":"parent_shutdown"}"#,
            ConnectionAuthority::BoundParent,
            DeliveryState::Idle,
            &writer,
        )
        .await;
    assert!(matches!(outcome, ControllerOutcome::Rejected { .. }));
    let frame = frame_json(&writer.frames()[0]);
    assert_eq!(frame["id"], "42");
    assert_eq!(
        frame["error"],
        "malformed teardown command: id must be a string"
    );
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
