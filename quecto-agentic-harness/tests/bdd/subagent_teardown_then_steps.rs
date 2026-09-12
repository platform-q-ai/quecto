//! Then-steps for the subagent teardown UDS edge and pure lifecycle
//! scenarios; state and helpers live in `subagent_teardown_steps`.

use cucumber::then;
use quecto::application::subagents::dto::{ReleaseOutcome, TerminationRouted};
use quecto::domain::ids::AgentUuid;
use quecto::domain::subagent_teardown::{LaunchGeneration, ShutdownReason};
use quecto::interface::uds::subagent_teardown::controller::ControllerOutcome;
use quecto::interface::uds::subagent_teardown::presenter::AckWriteError;

use crate::QuectoWorld;
use crate::common::teardown_fixture::{Call, identity};
use crate::subagent_teardown_steps::{last_frame, rig, state, writer};

// ---------------------------------------------------------------------------
// Then: UDS controller edge
// ---------------------------------------------------------------------------

#[then(expr = "the ACK for {string} was flushed before any teardown effect")]
fn then_ack_before_effects(world: &mut QuectoWorld, id: String) {
    let writer = writer(world);
    let frames = writer.frames();
    assert_eq!(frames.len(), 1, "exactly one ACK frame");
    assert_eq!(frames[0]["id"], id);
    assert_eq!(frames[0]["command"], "shutdown");
    assert_eq!(frames[0]["success"], true);
    assert_eq!(frames[0]["data"]["status"], "shutting_down");
    assert_eq!(*writer.effects_at_flush.lock().unwrap(), Some(0));
    assert!(matches!(
        world.teardown.controller_outcome,
        Some(ControllerOutcome::ShutdownExecuted { outcome: Ok(_), .. })
    ));
}

#[then(expr = "the ACK reports reason {string}")]
fn then_ack_reason(world: &mut QuectoWorld, reason: String) {
    assert_eq!(last_frame(world)["data"]["reason"], reason);
}

#[then(expr = "the shutdown is abandoned and the admission is {string}")]
fn then_abandoned(world: &mut QuectoWorld, release: String) {
    let expected = match release.as_str() {
        "released" => ReleaseOutcome::Released,
        "still held" => ReleaseOutcome::StillHeld,
        other => panic!("unknown release {other}"),
    };
    assert_eq!(
        world.teardown.controller_outcome,
        Some(ControllerOutcome::ShutdownAbandoned {
            ack_error: AckWriteError("broken pipe".into()),
            release: Ok(expected),
        })
    );
    assert!(
        writer(world).frames().is_empty(),
        "no frame reached the peer"
    );
}

#[then(expr = "the command is rejected with {string} correlated to {string}")]
fn then_rejected(world: &mut QuectoWorld, detail: String, id: String) {
    let Some(ControllerOutcome::Rejected {
        detail: actual,
        written: Ok(()),
        ..
    }) = &world.teardown.controller_outcome
    else {
        panic!(
            "expected a rejection, got {:?}",
            world.teardown.controller_outcome
        );
    };
    assert!(actual.contains(&detail), "{actual}");
    let frame = last_frame(world);
    if id == "null" {
        assert!(frame.get("id").is_none(), "no id to correlate: {frame}");
    } else {
        assert_eq!(frame["id"], id);
    }
    assert_eq!(frame["success"], false);
    assert!(frame["error"].as_str().unwrap().contains(&detail));
}

#[then("the line is ignored and no frame is written")]
fn then_ignored(world: &mut QuectoWorld) {
    assert_eq!(
        world.teardown.controller_outcome,
        Some(ControllerOutcome::Ignored)
    );
    assert!(writer(world).frames().is_empty());
}

#[then(expr = "the command is forwarded via {string} with remaining depth {int}")]
fn then_forwarded(world: &mut QuectoWorld, via: String, depth: u32) {
    let Some(ControllerOutcome::TerminationRouted {
        outcome:
            Ok(TerminationRouted::Forwarded {
                via: routed_via,
                remaining_depth,
            }),
        written: Ok(()),
        ..
    }) = &world.teardown.controller_outcome
    else {
        panic!(
            "expected forward, got {:?}",
            world.teardown.controller_outcome
        );
    };
    assert_eq!(routed_via.uuid, AgentUuid::new(&via));
    assert_eq!(remaining_depth.hops(), depth);
    let calls = rig(world).routing.calls();
    assert_eq!(calls.len(), 1);
    assert!(matches!(&calls[0], Call::Forward { via: v, .. } if v.uuid == AgentUuid::new(&via)));
}

#[then(expr = "child {string} is asked to shut down for {string}")]
fn then_child_shutdown(world: &mut QuectoWorld, child: String, reason: String) {
    let Some(ControllerOutcome::TerminationRouted {
        outcome: Ok(TerminationRouted::ShutdownRequested { child: routed }),
        written: Ok(()),
        ..
    }) = world.teardown.controller_outcome.clone()
    else {
        panic!(
            "expected shutdown request, got {:?}",
            world.teardown.controller_outcome
        );
    };
    assert_eq!(routed.uuid, AgentUuid::new(&child));
    assert_eq!(routed.generation, LaunchGeneration::new(1));
    let expected = ShutdownReason::parse(&reason).unwrap();
    assert_eq!(
        rig(world).routing.calls(),
        [Call::Shutdown(identity(&child, 1), expected)]
    );
}

#[then(expr = "the termination response for {string} has status {string}")]
fn then_termination_frame(world: &mut QuectoWorld, id: String, status: String) {
    let frame = last_frame(world);
    assert_eq!(frame["id"], id);
    assert_eq!(frame["command"], "terminate_delegated_agent");
    assert_eq!(frame["data"]["status"], status);
}

#[then(expr = "the termination is refused with {string} correlated to {string}")]
fn then_termination_refused(world: &mut QuectoWorld, detail: String, id: String) {
    let Some(ControllerOutcome::TerminationRouted {
        outcome: Err(error),
        written: Ok(()),
        ..
    }) = &world.teardown.controller_outcome
    else {
        panic!(
            "expected refusal, got {:?}",
            world.teardown.controller_outcome
        );
    };
    assert!(error.to_string().contains(&detail), "{error}");
    if !id.is_empty() {
        let frame = last_frame(world);
        assert_eq!(frame["id"], id);
        assert_eq!(frame["success"], false);
        assert!(frame["error"].as_str().unwrap().contains(&detail));
    }
}

#[then("no child was addressed")]
fn then_no_routing(world: &mut QuectoWorld) {
    assert!(rig(world).routing.calls().is_empty());
}

// ---------------------------------------------------------------------------
// Then: pure lifecycle
// ---------------------------------------------------------------------------

#[then(expr = "the lifecycle is {string}")]
fn then_pure_lifecycle(world: &mut QuectoWorld, name: String) {
    assert_eq!(world.teardown.lifecycle, Some(state(&name)));
    assert!(world.teardown.transition_error.is_none());
}

#[then(expr = "the transition is refused with {string}")]
fn then_transition_refused(world: &mut QuectoWorld, message: String) {
    let error = world.teardown.transition_error.expect("refused");
    assert_eq!(error.to_string(), message);
}

#[then(expr = "the lifecycle accepts new work: {string}")]
fn then_accepts(world: &mut QuectoWorld, flag: String) {
    let expected = match flag.as_str() {
        "yes" => true,
        "no" => false,
        other => panic!("unknown flag {other}"),
    };
    assert_eq!(
        world.teardown.lifecycle.unwrap().accepts_new_work(),
        expected
    );
}
