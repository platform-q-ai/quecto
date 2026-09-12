use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use super::*;
use crate::application::subagents::dto::ShutdownTrigger;
use crate::application::subagents::ports::{ExitReadiness, SubagentLifecycleRepository};
use crate::application::subagents::use_cases::teardown_fakes::*;
use crate::application::subagents::use_cases::{
    ExecuteHarnessShutdown, ExecuteHarnessShutdownPorts, HarnessShutdownTransaction,
    PrepareHarnessShutdown, TerminateDelegatedAgent,
};
use crate::domain::parent_control::{
    BindingState, ParentControlCapability, ParentControlCredential,
};
use crate::domain::subagent_teardown::{HarnessLifecycleState, LaunchGeneration, ShutdownReason};
use crate::infrastructure::processes::parent_control::presentation_json;
use crate::interface::uds::parent_control::wire::BindParentControlWire;

struct Rig {
    teardown: Arc<ConnectionTeardown>,
    cancellation: Arc<FakeCancellation>,
    routing: Arc<FakeRouting>,
    exit: Arc<FakeExit>,
    lifecycle: Arc<FakeLifecycle>,
    spawner: Arc<FakeSpawner>,
}

fn credential(generation: u64, byte: u8) -> ParentControlCredential {
    ParentControlCredential {
        generation: LaunchGeneration::new(generation),
        capability: ParentControlCapability::from_random_bytes(&[byte; 32]),
    }
}

fn rig_with(binding: ParentControlBinding, cancellation: Arc<FakeCancellation>) -> Rig {
    let lifecycle = FakeLifecycle::new(root_tree());
    let routing = FakeRouting::new();
    let exit = FakeExit::new();
    let spawner = FakeSpawner::new();
    let transaction = HarnessShutdownTransaction::new(lifecycle.clone(), FakeClock::at(1));
    let prepare = Arc::new(PrepareHarnessShutdown::new(transaction.clone()));
    let execute = Arc::new(ExecuteHarnessShutdown::new(
        transaction,
        ExecuteHarnessShutdownPorts {
            routing: routing.clone(),
            cancellation: cancellation.clone(),
            persistence: FakePersistence::new(),
            exit: exit.clone(),
            spawner: spawner.clone(),
        },
    ));
    let terminate = Arc::new(TerminateDelegatedAgent::new(
        lifecycle.clone(),
        routing.clone(),
    ));
    let controller = Arc::new(SubagentTeardownController::new(prepare, execute, terminate));
    Rig {
        teardown: Arc::new(ConnectionTeardown {
            binding: Arc::new(Mutex::new(binding)),
            controller,
            busy: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }),
        cancellation,
        routing,
        exit,
        lifecycle,
        spawner,
    }
}

fn rig(binding: ParentControlBinding) -> Rig {
    rig_with(binding, FakeCancellation::new(false))
}

/// A connection: the harness-side writer plus the peer's read half, so a
/// test can read what the harness wrote.
struct Conn {
    writer: SharedWriter,
    peer: tokio::net::UnixStream,
    role: ConnectionRole,
    mode: ConnectionWireMode,
    id: u64,
}

fn conn(id: u64) -> Conn {
    let (ours, peer) = tokio::net::UnixStream::pair().unwrap();
    let (_read, write) = tokio::io::split(ours);
    Conn {
        writer: Arc::new(tokio::sync::Mutex::new(write)),
        peer,
        role: ConnectionRole::default(),
        mode: ConnectionWireMode::legacy(),
        id,
    }
}

/// Every await on a shutdown or exit is bounded: a regression that hangs
/// fails fast instead of stalling the suite.
async fn bounded<F: std::future::Future>(future: F) -> F::Output {
    tokio::time::timeout(std::time::Duration::from_secs(10), future)
        .await
        .expect("bounded await timed out")
}

impl Conn {
    async fn line(&mut self, rig: &Rig, line: &str) -> Intercept {
        bounded(intercept_line(
            LineContext {
                teardown: &rig.teardown,
                role: &mut self.role,
                writer: &self.writer,
                wire_mode: &self.mode,
                client_id: self.id,
            },
            line,
        ))
        .await
    }

    async fn close(&self, rig: &Rig) -> Option<ControllerOutcome> {
        bounded(connection_closed(&rig.teardown, &self.role, self.id)).await
    }

    async fn read_line(&mut self) -> String {
        use tokio::io::AsyncBufReadExt;
        let mut reader = tokio::io::BufReader::new(&mut self.peer);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        line
    }
}

fn presentation(credential: &ParentControlCredential) -> String {
    presentation_json(credential)
}

impl Rig {
    fn nothing_ran(&self) {
        assert_eq!(self.spawner.spawned.load(Ordering::SeqCst), 0);
        assert_eq!(self.cancellation.calls.load(Ordering::SeqCst), 0);
        assert!(self.exit.signalled().is_empty());
        assert_eq!(self.lifecycle.lifecycle(), HarnessLifecycleState::Accepting);
    }

    fn ran_once(&self) {
        assert_eq!(self.spawner.spawned.load(Ordering::SeqCst), 1);
        assert_eq!(self.cancellation.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            self.exit.signalled(),
            [ExitReadiness::Completed(
                ShutdownReason::ParentConnectionLost
            )]
        );
        assert_eq!(
            self.lifecycle.lifecycle(),
            HarnessLifecycleState::Terminated
        );
        // Direct children A and D were asked to shut down, once each.
        let shutdowns: Vec<_> = self
            .routing
            .calls()
            .into_iter()
            .filter(|call| matches!(call, RoutingCall::Shutdown(..)))
            .collect();
        assert_eq!(shutdowns.len(), 2);
    }
}

#[tokio::test]
async fn presentation_fail_closed_matrix() {
    let expected = credential(3, 7);
    // (binding, presented line, why)
    let cases: Vec<(ParentControlBinding, String, &str)> = vec![
        (
            ParentControlBinding::unlaunched(),
            presentation(&expected),
            "a top-level harness has no parent",
        ),
        (
            ParentControlBinding::launched(expected.clone()),
            presentation(&credential(2, 7)),
            "wrong generation",
        ),
        (
            ParentControlBinding::launched(expected.clone()),
            presentation(&credential(3, 8)),
            "mismatched capability",
        ),
        (
            ParentControlBinding::launched(expected.clone()),
            r#"{"type":"bind_parent_control","generation":3}"#.to_string(),
            "malformed presentation",
        ),
        (
            ParentControlBinding::launched(expected.clone()),
            r#"{"type":"bind_parent_control","generation":3,"capability":"zz"}"#.to_string(),
            "capability not hex",
        ),
    ];
    for (binding, line, why) in cases {
        let rig = rig(binding);
        let mut c = conn(1);
        assert_eq!(c.line(&rig, &line).await, Intercept::Close, "{why}");
        assert!(!c.role.is_bound_parent(), "{why}");
        assert_ne!(rig.teardown.binding_state(), BindingState::Bound, "{why}");
        // The refused presenter's disconnect is an ordinary one.
        assert!(c.close(&rig).await.is_none(), "{why}");
        rig.nothing_ran();
    }
}

#[tokio::test]
async fn exactly_one_connection_binds_and_replays_close() {
    let expected = credential(1, 9);
    let rig = rig(ParentControlBinding::launched(expected.clone()));
    let mut parent = conn(1);
    assert_eq!(
        parent.line(&rig, &presentation(&expected)).await,
        Intercept::Handled
    );
    assert!(parent.role.is_bound_parent());
    assert_eq!(parent.role.authority(), ConnectionAuthority::BoundParent);
    assert_eq!(rig.teardown.binding_state(), BindingState::Bound);
    let ack: serde_json::Value =
        serde_json::from_str(bounded(parent.read_line()).await.trim()).unwrap();
    assert_eq!(ack["command"], "bind_parent_control");
    assert_eq!(ack["success"], true);
    // A replay on the same connection and a second connection presenting
    // the same (correct) capability both fail closed.
    assert_eq!(
        parent.line(&rig, &presentation(&expected)).await,
        Intercept::Close
    );
    let mut impostor = conn(2);
    assert_eq!(
        impostor.line(&rig, &presentation(&expected)).await,
        Intercept::Close
    );
    assert!(!impostor.role.is_bound_parent());
    assert_eq!(
        impostor.role.authority(),
        ConnectionAuthority::LocalOperator
    );
    // The bound one is still the parent.
    assert_eq!(rig.teardown.binding_state(), BindingState::Bound);
    rig.nothing_ran();
}

#[tokio::test]
async fn ordinary_lines_are_not_claimed_and_teardown_commands_carry_the_proven_authority() {
    let expected = credential(1, 1);
    let rig = rig(ParentControlBinding::launched(expected.clone()));
    let mut parent = conn(1);
    parent.line(&rig, &presentation(&expected)).await;
    assert_eq!(
        parent
            .line(&rig, r#"{"type":"prompt","message":"hi"}"#)
            .await,
        Intercept::NotClaimed
    );
    assert_eq!(parent.line(&rig, "not json").await, Intercept::NotClaimed);
    // A teardown command from an ordinary client is authorized as a local
    // operator today (no peer auth on the harness's own socket).
    let mut client = conn(2);
    let line = r#"{"type":"terminate_delegated_agent","id":"t","target_uuid":"D","target_generation":1,"remaining_depth":1}"#;
    assert_eq!(client.line(&rig, line).await, Intercept::Handled);
    let response: serde_json::Value =
        serde_json::from_str(client.read_line().await.trim()).unwrap();
    assert_eq!(response["id"], "t");
    assert_eq!(response["success"], true);
    assert_eq!(
        rig.routing.calls(),
        [RoutingCall::Shutdown(
            identity("D", 1),
            ShutdownReason::SelectedTermination
        )]
    );
}

#[tokio::test]
async fn bound_parent_eof_while_idle_runs_common_shutdown_exactly_once() {
    let expected = credential(4, 2);
    let rig = rig(ParentControlBinding::launched(expected.clone()));
    let mut parent = conn(1);
    parent.line(&rig, &presentation(&expected)).await;
    let outcome = parent.close(&rig).await.expect("the bound parent was lost");
    match outcome {
        ControllerOutcome::ShutdownExecuted { delivery, outcome } => {
            assert_eq!(delivery, DeliveryState::Idle);
            let outcome = outcome.unwrap();
            assert_eq!(outcome.reason, ShutdownReason::ParentConnectionLost);
            assert_eq!(outcome.triggers, [ShutdownTrigger::ParentConnectionClosed]);
            assert!(!outcome.turn_cancelled);
        }
        other => panic!("{other:?}"),
    }
    rig.ran_once();
    assert_eq!(rig.teardown.binding_state(), BindingState::Lost);
    // A repeated loss report of the same connection is inert.
    assert!(parent.close(&rig).await.is_none());
    rig.ran_once();
}

#[tokio::test]
async fn bound_parent_eof_during_a_running_turn_cancels_it_and_runs_once() {
    let expected = credential(5, 3);
    let rig = rig_with(
        ParentControlBinding::launched(expected.clone()),
        FakeCancellation::new(true),
    );
    rig.teardown.busy.store(true, Ordering::SeqCst);
    let mut parent = conn(1);
    parent.line(&rig, &presentation(&expected)).await;
    let outcome = parent.close(&rig).await.expect("bound parent lost");
    match outcome {
        ControllerOutcome::ShutdownExecuted { delivery, outcome } => {
            assert_eq!(delivery, DeliveryState::Busy);
            assert!(outcome.unwrap().turn_cancelled);
        }
        other => panic!("{other:?}"),
    }
    rig.ran_once();
}

#[tokio::test]
async fn bound_parent_eof_during_a_concurrent_spawn_converges_on_one_shutdown() {
    // The turn (a spawn in flight) holds the cancellation until released:
    // the shutdown is mid-flight while other connections come and go.
    let expected = credential(6, 4);
    let cancellation = FakeCancellation::holding();
    let rig = rig_with(
        ParentControlBinding::launched(expected.clone()),
        cancellation.clone(),
    );
    let mut parent = conn(1);
    parent.line(&rig, &presentation(&expected)).await;
    let teardown = rig.teardown.clone();
    let loss =
        tokio::spawn(async move { connection_closed(&teardown, &parent.role, parent.id).await });
    while cancellation.calls.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    assert_eq!(rig.lifecycle.lifecycle(), HarnessLifecycleState::Frozen);
    // Meanwhile the spawning tool's own client and the last inspector
    // disconnect: neither is the parent, nothing more is admitted.
    let spawner_client = conn(2);
    assert!(spawner_client.close(&rig).await.is_none());
    let last_inspector = conn(3);
    assert!(last_inspector.close(&rig).await.is_none());
    // A late presentation cannot re-bind a lost/bound parent either.
    let mut late = conn(4);
    assert_eq!(
        late.line(&rig, &presentation(&expected)).await,
        Intercept::Close
    );
    cancellation.gate.notify_one();
    let outcome = bounded(loss).await.unwrap().expect("bound parent lost");
    assert!(matches!(
        outcome,
        ControllerOutcome::ShutdownExecuted { .. }
    ));
    rig.ran_once();
}

#[tokio::test]
async fn non_parent_disconnects_including_the_last_client_never_invoke_shutdown() {
    let expected = credential(7, 5);
    let launched = rig(ParentControlBinding::launched(expected.clone()));
    let rig_ref = &launched;
    // Before any binding: clients come and go.
    let tui = conn(1);
    assert!(tui.close(rig_ref).await.is_none());
    // After the parent binds, an inspector, a tool client and a probe close.
    let mut parent = conn(2);
    parent.line(rig_ref, &presentation(&expected)).await;
    for id in 3..6 {
        let client = conn(id);
        assert!(client.close(rig_ref).await.is_none());
    }
    // A top-level harness: even its last client leaving is nothing.
    let top = rig(ParentControlBinding::unlaunched());
    let only = conn(9);
    assert!(only.close(&top).await.is_none());
    rig_ref.nothing_ran();
    top.nothing_ran();
    assert_eq!(rig_ref.teardown.binding_state(), BindingState::Bound);
}

#[tokio::test]
async fn a_terminated_harness_reports_the_loss_as_rejected_without_a_second_run() {
    let expected = credential(8, 6);
    let rig = rig(ParentControlBinding::launched(expected.clone()));
    rig.lifecycle.force(HarnessLifecycleState::Terminated);
    let mut parent = conn(1);
    parent.line(&rig, &presentation(&expected)).await;
    let outcome = parent.close(&rig).await.expect("bound parent lost");
    assert!(
        matches!(outcome, ControllerOutcome::Rejected { ref detail, .. } if detail.contains("terminated")),
        "{outcome:?}"
    );
    assert_eq!(rig.spawner.spawned.load(Ordering::SeqCst), 0);
    assert_eq!(redact(&outcome), "rejected");
    assert_eq!(redact(&ControllerOutcome::Ignored), "ignored");
}

#[test]
fn the_prefilter_keeps_ordinary_lines_away_from_the_parsers() {
    let big_prompt = format!(
        "{{\"type\":\"prompt\",\"message\":\"{}\"}}",
        "x".repeat(4 << 20)
    );
    assert!(!may_be_control_line(&big_prompt));
    assert!(!may_be_control_line("not json at all"));
    assert!(!may_be_control_line(""));
    for control in [
        r#"{"type":"bind_parent_control","generation":1,"capability":"aa"}"#,
        r#"{"type":"shutdown","reason":"parent_shutdown"}"#,
        r#"{"type":"terminate_delegated_agent","target_uuid":"x"}"#,
    ] {
        assert!(may_be_control_line(control), "{control}");
    }
}

#[tokio::test]
async fn a_large_ordinary_line_is_not_claimed_and_leaves_the_binding_untouched() {
    let expected = credential(2, 2);
    let rig = rig(ParentControlBinding::launched(expected));
    let mut c = conn(1);
    let big = format!(
        "{{\"type\":\"prompt\",\"message\":\"{}\"}}",
        "y".repeat(1 << 20)
    );
    assert_eq!(c.line(&rig, &big).await, Intercept::NotClaimed);
    assert_eq!(rig.teardown.binding_state(), BindingState::Unbound);
    rig.nothing_ran();
}

#[tokio::test]
async fn the_bind_deadline_ends_an_unbound_launched_harness_exactly_once() {
    let expected = credential(3, 3);
    let rig = rig(ParentControlBinding::launched(expected.clone()));
    let outcome = bounded(bind_deadline_passed(&rig.teardown))
        .await
        .expect("an unbound launched harness expires");
    match outcome {
        ControllerOutcome::ShutdownExecuted { outcome, .. } => {
            let outcome = outcome.unwrap();
            assert_eq!(outcome.reason, ShutdownReason::ParentNeverBound);
            assert_eq!(outcome.triggers, [ShutdownTrigger::ParentNeverBound]);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(rig.teardown.binding_state(), BindingState::Lost);
    assert_eq!(rig.spawner.spawned.load(Ordering::SeqCst), 1);
    assert_eq!(
        rig.exit.signalled(),
        [ExitReadiness::Completed(ShutdownReason::ParentNeverBound)]
    );
    // A second expiry and a late parent are both inert/refused.
    assert!(bounded(bind_deadline_passed(&rig.teardown)).await.is_none());
    let mut late = conn(1);
    assert_eq!(
        late.line(&rig, &presentation(&expected)).await,
        Intercept::Close
    );
    assert_eq!(rig.spawner.spawned.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn the_bind_deadline_is_inert_once_a_parent_bound_in_time() {
    let expected = credential(4, 4);
    let bound = rig(ParentControlBinding::launched(expected.clone()));
    let mut parent = conn(1);
    parent.line(&bound, &presentation(&expected)).await;
    let trigger = Arc::new(tokio::sync::Notify::new());
    let watcher = arm_bind_deadline(
        bound.teardown.clone(),
        super::super::uds_teardown_graph::BindDeadline::Triggered(trigger.clone()),
    );
    trigger.notify_one();
    assert!(bounded(watcher).await.unwrap().is_none());
    assert_eq!(bound.teardown.binding_state(), BindingState::Bound);
    bound.nothing_ran();
    // A top-level harness never expires either, and the timed variant is
    // the same watcher.
    let top = rig(ParentControlBinding::unlaunched());
    let watcher = arm_bind_deadline(
        top.teardown.clone(),
        super::super::uds_teardown_graph::BindDeadline::After(std::time::Duration::from_millis(5)),
    );
    assert!(bounded(watcher).await.unwrap().is_none());
    top.nothing_ran();
}

#[tokio::test]
async fn a_triggered_deadline_expires_an_unbound_harness_when_fired() {
    let rig = rig(ParentControlBinding::launched(credential(5, 5)));
    let trigger = Arc::new(tokio::sync::Notify::new());
    let watcher = arm_bind_deadline(
        rig.teardown.clone(),
        super::super::uds_teardown_graph::BindDeadline::Triggered(trigger.clone()),
    );
    tokio::task::yield_now().await;
    assert_eq!(rig.teardown.binding_state(), BindingState::Unbound);
    trigger.notify_one();
    let outcome = bounded(watcher).await.unwrap().expect("expired");
    assert!(matches!(
        outcome,
        ControllerOutcome::ShutdownExecuted { .. }
    ));
    assert_eq!(rig.teardown.binding_state(), BindingState::Lost);
}

#[test]
fn json_escaped_command_types_still_reach_the_parsers() {
    for escaped in [
        r#"{"type":"sh\u0075tdown","reason":"parent_shutdown"}"#,
        r#"{"type":"terminate_delegated_\u0061gent","target_uuid":"x"}"#,
        r#"{"type":"bind_parent_c\u006fntrol","generation":1,"capability":"aa"}"#,
    ] {
        assert!(may_be_control_line(escaped), "{escaped}");
    }
    assert!(BindParentControlWire::may_be_presentation(
        r#"{"type":"bind_parent_c\u006fntrol"}"#
    ));
    // Any `\u00` escape is parsed in full (and then not claimed): the cost
    // is paid only by lines that carry such an escape.
    assert!(may_be_control_line(
        r#"{"type":"prompt","message":"caf\u00e9"}"#
    ));
}

#[tokio::test]
async fn json_escaped_spellings_are_claimed_and_fail_closed_like_plain_ones() {
    let expected = credential(9, 9);
    let rig = rig(ParentControlBinding::launched(expected.clone()));
    // Escaped presentation with the right material binds.
    let mut parent = conn(1);
    let escaped_bind =
        presentation(&expected).replace("bind_parent_control", "bind_parent_c\\u006fntrol");
    assert!(escaped_bind.contains("\\u006f"));
    assert_eq!(parent.line(&rig, &escaped_bind).await, Intercept::Handled);
    assert!(parent.role.is_bound_parent());
    // Escaped shutdown is claimed by the controller (rejected: bad reason),
    // never dispatched as an ordinary command.
    let mut client = conn(2);
    let escaped_shutdown = r#"{"type":"sh\u0075tdown","id":"e-1","reason":"nope"}"#;
    assert_eq!(
        client.line(&rig, escaped_shutdown).await,
        Intercept::Handled
    );
    let response: serde_json::Value =
        serde_json::from_str(bounded(client.read_line()).await.trim()).unwrap();
    assert_eq!(response["id"], "e-1");
    assert_eq!(response["success"], false);
    let escaped_terminate = r#"{"type":"terminate_delegated_\u0061gent","id":"e-2","target_uuid":"ghost","target_generation":1,"remaining_depth":1}"#;
    assert_eq!(
        client.line(&rig, escaped_terminate).await,
        Intercept::Handled
    );
    let response: serde_json::Value =
        serde_json::from_str(bounded(client.read_line()).await.trim()).unwrap();
    assert_eq!(response["id"], "e-2");
    assert_eq!(response["success"], false);
    rig.nothing_ran();
}
