//! BDD steps for the subagent teardown contracts (#1934): the two-phase
//! shutdown transaction, selected-descendant routing, the UDS controller
//! edge, and the pure lifecycle invariants. Everything runs against the
//! public crate surface with the shared port fakes.
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use cucumber::{given, then, when};
use quecto::application::subagents::dto::{
    HarnessShutdownError, PrepareShutdownRequest, PreparedShutdown, ReleaseOutcome,
    ShutdownOutcome, ShutdownTrigger, TerminateDelegatedAgentRequest,
};
use quecto::application::subagents::ports::SubagentLifecycleRepository;
use quecto::application::subagents::use_cases::{
    ExecuteHarnessShutdown, ExecuteHarnessShutdownPorts, HarnessShutdownTransaction,
    PrepareHarnessShutdown, TerminateDelegatedAgent,
};
use quecto::domain::ids::AgentUuid;
use quecto::domain::subagent_teardown::{
    HarnessLifecycleState, LifecycleTransitionError, LineageSnapshot, RoutingDepth, ShutdownReason,
};
use quecto::interface::uds::subagent_teardown::controller::{
    ConnectionAuthority, ControllerOutcome, DeliveryState, SubagentTeardownController,
};
use quecto::interface::uds::subagent_teardown::presenter::{AckWriteError, AckWriter};
use quecto::interface::uds::subagent_teardown::wire::{
    TEARDOWN_COMMAND_CAP_BYTES, TeardownResponse,
};

use crate::QuectoWorld;
use crate::common::teardown_fixture::{
    Call, Cancellation, Clock, Exit, Lifecycle, Persistence, Routing, Spawner, identity, record,
    root_tree,
};

// ---------------------------------------------------------------------------
// ACK writer fake
// ---------------------------------------------------------------------------

#[derive(Default)]
pub(crate) struct Writer {
    pub(crate) frames: Mutex<Vec<String>>,
    pub(crate) fail: AtomicBool,
    /// Number of teardown effects observed when the ACK was flushed.
    pub(crate) effects_at_flush: Mutex<Option<usize>>,
    pub(crate) effects_probe: Mutex<Option<Arc<Cancellation>>>,
}

impl Writer {
    pub(crate) fn frames(&self) -> Vec<serde_json::Value> {
        self.frames
            .lock()
            .unwrap()
            .iter()
            .map(|frame| serde_json::from_str(frame.trim_end()).expect("frame is json"))
            .collect()
    }
}

impl AckWriter for Writer {
    fn write_and_flush<'a>(
        &'a self,
        response: &'a TeardownResponse,
    ) -> Pin<Box<dyn Future<Output = Result<(), AckWriteError>> + Send + 'a>> {
        Box::pin(async move {
            if self.fail.load(Ordering::SeqCst) {
                return Err(AckWriteError("broken pipe".into()));
            }
            // This fake frames as the legacy newline-delimited line.
            let line = response.to_line();
            assert!(line.ends_with('\n'), "frames are newline-terminated");
            let effects = self
                .effects_probe
                .lock()
                .unwrap()
                .as_ref()
                .map(|probe| probe.calls.load(Ordering::SeqCst));
            *self.effects_at_flush.lock().unwrap() = effects;
            self.frames.lock().unwrap().push(line.to_owned());
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// Scenario state
// ---------------------------------------------------------------------------

pub(crate) struct Rig {
    pub(crate) lifecycle: Arc<Lifecycle>,
    pub(crate) routing: Arc<Routing>,
    pub(crate) cancellation: Arc<Cancellation>,
    pub(crate) persistence: Arc<Persistence>,
    pub(crate) exit: Arc<Exit>,
    pub(crate) spawner: Arc<Spawner>,
    pub(crate) transaction: Arc<HarnessShutdownTransaction>,
    pub(crate) prepare: Arc<PrepareHarnessShutdown>,
    pub(crate) execute: Arc<ExecuteHarnessShutdown>,
    pub(crate) controller: Arc<SubagentTeardownController>,
}

impl Rig {
    pub(crate) fn new(lineage: LineageSnapshot) -> Self {
        let lifecycle = Lifecycle::new(lineage);
        let routing = Routing::new();
        let cancellation = Cancellation::with_turn();
        let persistence = Arc::new(Persistence::default());
        let exit = Arc::new(Exit::default());
        let spawner = Arc::new(Spawner::default());
        let clock = Arc::new(Clock(std::sync::atomic::AtomicU64::new(1_000)));
        let transaction = HarnessShutdownTransaction::new(lifecycle.clone(), clock);
        let prepare = Arc::new(PrepareHarnessShutdown::new(transaction.clone()));
        let execute = Arc::new(ExecuteHarnessShutdown::new(
            transaction.clone(),
            ExecuteHarnessShutdownPorts {
                routing: routing.clone(),
                cancellation: cancellation.clone(),
                persistence: persistence.clone(),
                exit: exit.clone(),
                spawner: spawner.clone(),
            },
        ));
        let terminate = Arc::new(TerminateDelegatedAgent::new(
            lifecycle.clone(),
            routing.clone(),
        ));
        let controller = Arc::new(SubagentTeardownController::new(
            prepare.clone(),
            execute.clone(),
            terminate,
        ));
        Self {
            lifecycle,
            routing,
            cancellation,
            persistence,
            exit,
            spawner,
            transaction,
            prepare,
            execute,
            controller,
        }
    }

    pub(crate) fn effect_count(&self) -> usize {
        self.cancellation.calls.load(Ordering::SeqCst)
            + self.routing.calls().len()
            + self.persistence.calls.lock().unwrap().len()
            + self.exit.signalled.lock().unwrap().len()
    }
}

#[derive(Default)]
pub struct TeardownState {
    pub(crate) rig: Option<Rig>,
    pub(crate) writer: Option<Arc<Writer>>,
    pub(crate) prepared: Vec<PreparedShutdown>,
    pub(crate) outcome: Option<Result<ShutdownOutcome, HarnessShutdownError>>,
    pub(crate) joiner: Option<Result<ShutdownOutcome, HarnessShutdownError>>,
    pub(crate) release: Option<Result<ReleaseOutcome, HarnessShutdownError>>,
    pub(crate) controller_outcome: Option<ControllerOutcome>,
    pub(crate) lifecycle: Option<HarnessLifecycleState>,
    pub(crate) transition_error: Option<LifecycleTransitionError>,
}

impl std::fmt::Debug for TeardownState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<subagent teardown scenario>")
    }
}

pub(crate) fn rig(world: &mut QuectoWorld) -> &Rig {
    world
        .teardown
        .rig
        .as_ref()
        .expect("scenario must start with a harness")
}

pub(crate) fn writer(world: &mut QuectoWorld) -> Arc<Writer> {
    world
        .teardown
        .writer
        .get_or_insert_with(|| Arc::new(Writer::default()))
        .clone()
}

/// Drive one step's future to completion on a single-threaded runtime
/// (so `yield_now` deterministically lets spawned tasks run) under a bounded
/// timeout, so a hang is a failed step rather than a wedged cucumber
/// executor.
pub(crate) fn drive<F: std::future::Future>(future: F) -> F::Output {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    // The timeout must be built inside the runtime context (it needs the
    // timer driver), so wrap it in the driven future.
    runtime
        .block_on(async { tokio::time::timeout(STEP_TIMEOUT, future).await })
        .expect("teardown step exceeded its 10 s bound")
}

const STEP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

pub(crate) fn trigger(name: &str) -> ShutdownTrigger {
    match name {
        "protocol" => ShutdownTrigger::ProtocolCommand,
        "parent-closed" => ShutdownTrigger::ParentConnectionClosed,
        "signal" => ShutdownTrigger::TerminationSignal,
        other => panic!("unknown trigger {other}"),
    }
}

pub(crate) fn authority(name: &str) -> ConnectionAuthority {
    match name {
        "bound parent" => ConnectionAuthority::BoundParent,
        "local operator" => ConnectionAuthority::LocalOperator,
        "unauthenticated client" => ConnectionAuthority::Unauthenticated,
        other => panic!("unknown authority {other}"),
    }
}

pub(crate) fn delivery(name: &str) -> DeliveryState {
    match name {
        "idle" => DeliveryState::Idle,
        "busy" => DeliveryState::Busy,
        other => panic!("unknown delivery state {other}"),
    }
}

pub(crate) fn state(name: &str) -> HarnessLifecycleState {
    match name {
        "Accepting" => HarnessLifecycleState::Accepting,
        "Frozen" => HarnessLifecycleState::Frozen,
        "Terminated" => HarnessLifecycleState::Terminated,
        other => panic!("unknown lifecycle state {other}"),
    }
}

pub(crate) fn uuids(list: &str) -> Vec<AgentUuid> {
    list.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(AgentUuid::new)
        .collect()
}

pub(crate) fn handle(world: &mut QuectoWorld, line: &str, who: &str, how: &str) {
    let writer = writer(world);
    let rig = rig(world);
    *writer.effects_probe.lock().unwrap() = Some(rig.cancellation.clone());
    let outcome =
        drive(
            rig.controller
                .handle(line, authority(who), delivery(how), writer.as_ref()),
        );
    world.teardown.controller_outcome = Some(outcome);
}

pub(crate) fn last_frame(world: &mut QuectoWorld) -> serde_json::Value {
    writer(world)
        .frames()
        .last()
        .cloned()
        .expect("a frame was written")
}

// ---------------------------------------------------------------------------
// Given
// ---------------------------------------------------------------------------

#[given("a harness owning children A and D, where A owns B and C")]
fn given_root_tree(world: &mut QuectoWorld) {
    world.teardown.rig = Some(Rig::new(root_tree()));
}

#[given(expr = "session persistence fails with {string}")]
fn given_persistence_fails(world: &mut QuectoWorld, detail: String) {
    *rig(world).persistence.fail_with.lock().unwrap() = Some(detail);
}

#[then(expr = "executing a token from another harness fails with {string}")]
fn then_execute_foreign_fails(world: &mut QuectoWorld, message: String) {
    let foreign = Rig::new(root_tree());
    let token = foreign
        .prepare
        .execute(PrepareShutdownRequest {
            reason: ShutdownReason::OperatorRequest,
            trigger: ShutdownTrigger::ProtocolCommand,
        })
        .unwrap()
        .token;
    let rig = rig(world);
    let error = drive(rig.execute.execute(&token)).expect_err("foreign token");
    assert_eq!(error.to_string(), message);
}

#[then(expr = "the outcome reports child {string} failed and persistence failed with {string}")]
fn then_partial_failures(world: &mut QuectoWorld, child: String, detail: String) {
    let outcome = world
        .teardown
        .outcome
        .clone()
        .expect("executed")
        .expect("execute succeeded");
    assert_eq!(outcome.children_failed.len(), 1);
    assert_eq!(outcome.children_failed[0].0, AgentUuid::new(&child));
    assert_eq!(
        outcome.persistence,
        quecto::application::subagents::dto::PersistenceOutcome::Failed(detail)
    );
    assert!(outcome.exit_signalled);
}

#[then("every canonical shutdown reason round-trips and \"kill\" is unknown")]
fn then_reason_vocabulary(_world: &mut QuectoWorld) {
    for reason in ShutdownReason::ALL {
        assert_eq!(ShutdownReason::parse(&reason.to_string()), Ok(reason));
    }
    let unknown = ShutdownReason::parse("kill").expect_err("kill is not a reason");
    assert_eq!(unknown.to_string(), "unknown shutdown reason \"kill\"");
    assert_eq!(
        AckWriteError("broken pipe".into()).to_string(),
        "ack write failed: broken pipe"
    );
}

#[given("a harness whose lineage records X under Y and Y under X")]
fn given_cyclic_tree(world: &mut QuectoWorld) {
    world.teardown.rig = Some(Rig::new(LineageSnapshot {
        owner: AgentUuid::new("root"),
        records: vec![record("X", 1, "Y"), record("Y", 1, "X")],
    }));
}

#[given("a harness whose lineage lists B under both A and D, and C under B")]
fn given_ambiguous_tree(world: &mut QuectoWorld) {
    world.teardown.rig = Some(Rig::new(LineageSnapshot {
        owner: AgentUuid::new("root"),
        records: vec![
            record("A", 1, "root"),
            record("D", 1, "root"),
            record("B", 1, "D"),
            record("B", 2, "A"),
            record("C", 1, "B"),
        ],
    }));
}

#[given("a harness whose lineage records X under a parent it has never seen")]
fn given_dangling_tree(world: &mut QuectoWorld) {
    world.teardown.rig = Some(Rig::new(LineageSnapshot {
        owner: AgentUuid::new("root"),
        records: vec![record("X", 1, "ghost")],
    }));
}

#[given("the lifecycle repository loses the freeze behind the transaction's back")]
fn given_lost_freeze(world: &mut QuectoWorld) {
    rig(world).lifecycle.force(HarnessLifecycleState::Accepting);
}

#[given("the harness has already terminated")]
fn given_terminated(world: &mut QuectoWorld) {
    rig(world)
        .lifecycle
        .force(HarnessLifecycleState::Terminated);
}

#[given("the ACK writer cannot flush")]
fn given_failing_writer(world: &mut QuectoWorld) {
    writer(world).fail.store(true, Ordering::SeqCst);
}

#[given("the in-flight turn holds cancellation open")]
fn given_holding_cancellation(world: &mut QuectoWorld) {
    rig(world).cancellation.hold.store(true, Ordering::SeqCst);
}

#[given(expr = "child {string} is unreachable")]
fn given_unreachable(world: &mut QuectoWorld, child: String) {
    rig(world)
        .routing
        .unreachable
        .lock()
        .unwrap()
        .push(AgentUuid::new(child));
}

#[given(expr = "a harness lifecycle of {string}")]
fn given_lifecycle(world: &mut QuectoWorld, name: String) {
    world.teardown.lifecycle = Some(state(&name));
    world.teardown.transition_error = None;
}

// ---------------------------------------------------------------------------
// When: application transaction
// ---------------------------------------------------------------------------

#[when(expr = "shutdown is prepared for {string} by the {string} trigger")]
fn when_prepared(world: &mut QuectoWorld, reason: String, by: String) {
    let request = PrepareShutdownRequest {
        reason: ShutdownReason::parse(&reason).expect("canonical reason"),
        trigger: trigger(&by),
    };
    let prepared = rig(world).prepare.execute(request).expect("prepare admits");
    world.teardown.prepared.push(prepared);
}

#[when("the admitted token is executed")]
fn when_executed(world: &mut QuectoWorld) {
    let token = world.teardown.prepared[0].token.clone();
    let rig = rig(world);
    let outcome = drive(rig.execute.execute(&token));
    world.teardown.outcome = Some(outcome);
}

#[when(expr = "holder {int} releases its token")]
fn when_holder_released(world: &mut QuectoWorld, holder: usize) {
    let token = world.teardown.prepared[holder - 1].token.clone();
    world.teardown.release = Some(rig(world).prepare.release(&token));
}

#[when("the admitted token is released")]
fn when_released(world: &mut QuectoWorld) {
    when_holder_released(world, 1);
}

#[when("the detached run is dropped by its runtime while a joiner waits")]
fn when_run_dropped(world: &mut QuectoWorld) {
    let token = world.teardown.prepared[0].token.clone();
    let rig = rig(world);
    let joiner = drive(async {
        let starter = tokio::spawn({
            let execute = rig.execute.clone();
            let token = token.clone();
            async move { execute.execute(&token).await }
        });
        while rig.cancellation.calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        // A second caller joins the run already in flight.
        let joiner = tokio::spawn({
            let execute = rig.execute.clone();
            let token = token.clone();
            async move { execute.execute(&token).await }
        });
        // Make the joiner's registration observable before interrupting the
        // run: otherwise an unpolled joiner could admit a second run after
        // the guard hands the admission back.
        while rig.transaction.waiting_joiners() < 2 {
            tokio::task::yield_now().await;
        }
        assert!(
            !starter.is_finished(),
            "starter waits on the running teardown"
        );
        assert!(
            !joiner.is_finished(),
            "joiner waits on the running teardown"
        );
        rig.spawner.abort_latest().await;
        let starter = starter.await.unwrap();
        let joiner = joiner.await.unwrap();
        assert_eq!(starter, joiner, "both callers see the interruption");
        joiner
    });
    world.teardown.joiner = Some(joiner);
}

#[when("the spawner drops the next run unpolled and the token is executed")]
fn when_spawner_drops(world: &mut QuectoWorld) {
    rig(world).spawner.drop_next.store(1, Ordering::SeqCst);
    let token = world.teardown.prepared[0].token.clone();
    let rig = rig(world);
    world.teardown.joiner = Some(drive(rig.execute.execute(&token)));
}

#[when("cancellation is released and the latest token is executed")]
fn when_latest_executed_after_release(world: &mut QuectoWorld) {
    rig(world).cancellation.hold.store(false, Ordering::SeqCst);
    let token = world
        .teardown
        .prepared
        .last()
        .expect("a token was prepared")
        .token
        .clone();
    let rig = rig(world);
    let outcome = drive(rig.execute.execute(&token));
    world.teardown.outcome = Some(outcome);
}

#[when("cancellation is released and the same token is executed again")]
fn when_executed_after_release(world: &mut QuectoWorld) {
    rig(world).cancellation.hold.store(false, Ordering::SeqCst);
    when_executed(world);
}

#[when("the connection task is aborted after the ACK is flushed")]
fn when_task_aborted_after_ack(world: &mut QuectoWorld) {
    let writer = writer(world);
    let rig = rig(world);
    rig.cancellation.hold.store(true, Ordering::SeqCst);
    drive(async {
        let task = tokio::spawn({
            let controller = rig.controller.clone();
            let writer = writer.clone();
            async move {
                controller
                    .handle(
                        r#"{"type":"shutdown","id":"c-9","reason":"parent_shutdown"}"#,
                        ConnectionAuthority::BoundParent,
                        DeliveryState::Busy,
                        writer.as_ref(),
                    )
                    .await
            }
        });
        while rig.cancellation.calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        assert_eq!(writer.frames().len(), 1, "the ACK is on the wire");
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        rig.cancellation.gate.notify_one();
        rig.spawner.latest_finished().await;
    });
}

// ---------------------------------------------------------------------------
// When: UDS controller edge
// ---------------------------------------------------------------------------

#[when(expr = "the {string} sends shutdown {string} with id {string} to the {string} harness")]
fn when_shutdown_command(
    world: &mut QuectoWorld,
    who: String,
    reason: String,
    id: String,
    how: String,
) {
    let line = serde_json::json!({ "type": "shutdown", "id": id, "reason": reason }).to_string();
    handle(world, &line, &who, &how);
}

#[when(
    expr = "the {string} sends terminate_delegated_agent for {string} generation {int} depth {int} with id {string}"
)]
fn when_terminate_command(
    world: &mut QuectoWorld,
    who: String,
    target: String,
    generation: u64,
    depth: u32,
    id: String,
) {
    let line = serde_json::json!({
        "type": "terminate_delegated_agent",
        "id": id,
        "target_uuid": target,
        "target_generation": generation,
        "remaining_depth": depth,
    })
    .to_string();
    handle(world, &line, &who, "idle");
}

#[when(expr = "the bound parent sends the raw line {string}")]
fn when_raw_line(world: &mut QuectoWorld, line: String) {
    // Cucumber keeps the `\"` escapes of the quoted step argument.
    let line = line.replace("\\\"", "\"");
    handle(world, &line, "bound parent", "idle");
}

#[when("the bound parent sends a shutdown line one byte over the teardown cap")]
fn when_oversized(world: &mut QuectoWorld) {
    let prefix = r#"{"type":"shutdown","id":"big","reason":""#;
    let suffix = r#""}"#;
    let padding = "x".repeat(TEARDOWN_COMMAND_CAP_BYTES + 1 - prefix.len() - suffix.len());
    let line = format!("{prefix}{padding}{suffix}");
    assert_eq!(line.len(), TEARDOWN_COMMAND_CAP_BYTES + 1);
    handle(world, &line, "bound parent", "busy");
}

#[when("selected termination of B with depth 2 is requested through the use case")]
fn when_terminate_use_case(world: &mut QuectoWorld) {
    let rig = rig(world);
    let use_case = TerminateDelegatedAgent::new(rig.lifecycle.clone(), rig.routing.clone());
    let result = drive(use_case.execute(TerminateDelegatedAgentRequest {
        target: identity("B", 1),
        remaining_depth: RoutingDepth::new(2).unwrap(),
    }));
    world.teardown.controller_outcome = Some(ControllerOutcome::TerminationRouted {
        delivery: DeliveryState::Idle,
        outcome: result,
        written: Ok(()),
    });
}

// ---------------------------------------------------------------------------
// When: pure lifecycle
// ---------------------------------------------------------------------------

#[when(expr = "the lifecycle is asked to {string}")]
fn when_lifecycle_transition(world: &mut QuectoWorld, verb: String) {
    let current = world.teardown.lifecycle.expect("lifecycle staged");
    let result = match verb.as_str() {
        "freeze" => current.freeze(),
        "thaw" => current.thaw(),
        "terminate" => current.terminate(),
        other => panic!("unknown transition {other}"),
    };
    match result {
        Ok(next) => world.teardown.lifecycle = Some(next),
        Err(error) => world.teardown.transition_error = Some(error),
    }
}

// ---------------------------------------------------------------------------
// Then: application transaction
// ---------------------------------------------------------------------------

#[then(expr = "the harness lifecycle is {string}")]
fn then_lifecycle(world: &mut QuectoWorld, name: String) {
    assert_eq!(rig(world).lifecycle.lifecycle(), state(&name));
    assert_eq!(
        rig(world).transaction.accepts_new_work(),
        state(&name).accepts_new_work()
    );
}

#[then("no teardown effect has run")]
fn then_no_effect(world: &mut QuectoWorld) {
    assert_eq!(
        rig(world).effect_count(),
        0,
        "no port must have been touched"
    );
}

#[then(
    expr = "the teardown cancelled the turn, shut down {string}, persisted once and signalled exit once"
)]
fn then_full_teardown(world: &mut QuectoWorld, children: String) {
    let rig = rig(world);
    assert_eq!(rig.cancellation.calls.load(Ordering::SeqCst), 1);
    let shut: Vec<_> = rig
        .routing
        .calls()
        .into_iter()
        .map(|call| match call {
            Call::Shutdown(child, ShutdownReason::ParentShutdown) => child.uuid,
            other => panic!("unexpected routing call {other:?}"),
        })
        .collect();
    assert_eq!(shut, uuids(&children));
    assert_eq!(rig.persistence.calls.lock().unwrap().len(), 1);
    assert_eq!(rig.exit.signalled.lock().unwrap().len(), 1);
}

#[then("both preparations hold one admission and the second joined with its own token")]
fn then_joined(world: &mut QuectoWorld) {
    let prepared = &world.teardown.prepared;
    assert_eq!(prepared.len(), 2);
    assert!(!prepared[0].joined);
    assert!(prepared[1].joined);
    assert_ne!(prepared[0].token, prepared[1].token);
    assert_eq!(format!("{:?}", prepared[1].token), "ShutdownToken(..)");
    assert_eq!(prepared[1].reason, prepared[0].reason);
}

#[then(expr = "executing holder {int}'s token fails with {string}")]
fn then_execute_holder_fails(world: &mut QuectoWorld, holder: usize, message: String) {
    let token = world.teardown.prepared[holder - 1].token.clone();
    let rig = rig(world);
    let error = drive(rig.execute.execute(&token)).expect_err("spent token");
    assert_eq!(error.to_string(), message);
}

#[then(expr = "the shutdown outcome records reason {string} and triggers {string}")]
fn then_outcome(world: &mut QuectoWorld, reason: String, triggers: String) {
    let outcome = world
        .teardown
        .outcome
        .clone()
        .expect("executed")
        .expect("execute succeeded");
    assert_eq!(outcome.reason, ShutdownReason::parse(&reason).unwrap());
    let expected: Vec<_> = triggers.split(',').map(str::trim).map(trigger).collect();
    assert_eq!(outcome.triggers, expected);
    assert!(outcome.turn_cancelled);
    assert!(outcome.exit_signalled);
}

#[then(expr = "the release outcome is {string}")]
fn then_release(world: &mut QuectoWorld, expected: String) {
    let actual = world.teardown.release.clone().expect("released");
    let expected = match expected.as_str() {
        "released" => Ok(ReleaseOutcome::Released),
        "still held" => Ok(ReleaseOutcome::StillHeld),
        "already released" => Ok(ReleaseOutcome::AlreadyReleased),
        "execution underway" => Ok(ReleaseOutcome::ExecutionUnderway),
        "not prepared" => Err(HarnessShutdownError::NotPrepared),
        other => panic!("unknown release outcome {other}"),
    };
    assert_eq!(actual, expected);
}

#[then(expr = "the joiner is released with {string}")]
fn then_joiner(world: &mut QuectoWorld, message: String) {
    let joiner = world.teardown.joiner.clone().expect("joiner ran");
    let error = joiner.expect_err("joiner reports the interruption");
    assert_eq!(error, HarnessShutdownError::ExecutionInterrupted);
    assert_eq!(error.to_string(), message);
}

#[then(expr = "the execution fails with {string}")]
fn then_execution_fails(world: &mut QuectoWorld, message: String) {
    let outcome = world.teardown.outcome.clone().expect("executed");
    assert_eq!(outcome.expect_err("execution failed").to_string(), message);
}

#[then(expr = "the executed outcome shut down {string}")]
fn then_outcome_children(world: &mut QuectoWorld, children: String) {
    let outcome = world
        .teardown
        .outcome
        .clone()
        .expect("executed")
        .expect("execute succeeded");
    assert_eq!(outcome.children_shut_down, uuids(&children));
}

#[then("a later shutdown preparation joins the completed outcome")]
fn then_late_prepare_joins(world: &mut QuectoWorld) {
    let first = world.teardown.prepared[0].clone();
    let late = rig(world)
        .prepare
        .execute(PrepareShutdownRequest {
            reason: ShutdownReason::OperatorRequest,
            trigger: ShutdownTrigger::ParentConnectionClosed,
        })
        .expect("joins");
    assert!(late.joined);
    assert_ne!(late.token, first.token);
    assert_eq!(late.reason, first.reason);
    let expected = world.teardown.outcome.clone().unwrap();
    let rig = rig(world);
    let again = drive(rig.execute.execute(&late.token));
    assert_eq!(again, expected);
    assert_eq!(
        rig.prepare.release(&late.token),
        Ok(ReleaseOutcome::ExecutionUnderway)
    );
}
