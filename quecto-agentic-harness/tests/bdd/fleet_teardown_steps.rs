//! BDD steps for the fleet teardown (#1938): the `TerminateAllDelegatedAgents`
//! use case over port fakes, the harness shutdown driving it, spawn admission
//! against a frozen harness, and the busy-path `delete_all_subagents` over
//! the production adapters. The real-process entry points (signal, last
//! client, session transitions) live in `restore_lifetime_steps`.
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use cucumber::{given, then, when};
use quecto::application::subagents::dto::{
    FleetChildResult, FleetTeardownError, FleetTeardownOutcome, ShutdownOutcome,
    TerminateAllDelegatedAgentsRequest,
};
use quecto::application::subagents::ports::{
    DelegatedAgentRegistry, SubagentLifecycleRepository, TerminationCause, TerminationConclusion,
};
use quecto::domain::ids::AgentUuid;
use quecto::domain::subagent_teardown::{HarnessLifecycleState, ShutdownReason};

use crate::QuectoWorld;
use crate::common::teardown_fixture::{Harness, RowPhase, root_tree};

#[derive(Default)]
pub(crate) struct FleetTeardownState {
    harness: Option<Harness>,
    runtime: Option<tokio::runtime::Runtime>,
    outcomes: Vec<Result<FleetTeardownOutcome, FleetTeardownError>>,
    shutdown: Option<ShutdownOutcome>,
    spawn_refusal: Option<String>,
    busy_response: Option<serde_json::Value>,
    busy_registry: Option<quecto::infrastructure::tools::subagent_registry::SubagentRegistry>,
}

impl std::fmt::Debug for FleetTeardownState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FleetTeardownState")
    }
}

fn state(world: &mut QuectoWorld) -> &mut FleetTeardownState {
    &mut world.fleet_teardown
}

fn runtime(world: &mut QuectoWorld) -> tokio::runtime::Runtime {
    state(world).runtime.take().unwrap_or_else(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
    })
}

fn harness(world: &mut QuectoWorld) -> &Harness {
    state(world).harness.as_ref().expect("a fleet harness")
}

fn bounded<F: std::future::Future>(rt: &tokio::runtime::Runtime, future: F) -> F::Output {
    rt.block_on(async {
        tokio::time::timeout(Duration::from_secs(10), future)
            .await
            .expect("bounded await timed out")
    })
}

fn request() -> TerminateAllDelegatedAgentsRequest {
    // The operator's delete-all: the owner's explicit word (#2070).
    TerminateAllDelegatedAgentsRequest {
        reason: ShutdownReason::OperatorRequest,
        authority: quecto::application::subagents::dto::FleetTeardownAuthority::Owner,
    }
}

// ── Given ────────────────────────────────────────────────────────────────────

#[given("a harness owning direct children A and D with A's own children B and C")]
fn given_fleet(world: &mut QuectoWorld) {
    let rt = runtime(world);
    let harness = rt.block_on(async { Harness::new(root_tree()) });
    state(world).harness = Some(harness);
    state(world).runtime = Some(rt);
}

#[given(expr = "child {string} survives even the owned-handle fallback")]
fn given_survivor(world: &mut QuectoWorld, child: String) {
    harness(world).rows.conclusion_for.lock().unwrap().insert(
        child.clone(),
        TerminationConclusion::StillRunning(format!("{child} ignores TERM and KILL")),
    );
}

#[given(expr = "an operator kill already claimed child {string}")]
fn given_kill_claimed(world: &mut QuectoWorld, child: String) {
    let harness = harness(world);
    let identity = harness.rows.resolve(&child).unwrap();
    harness
        .rows
        .claim_stopping(&identity, TerminationCause::SelectedTermination)
        .unwrap();
}

#[given(expr = "the shutdown of child {string} is held open")]
fn given_held(world: &mut QuectoWorld, child: String) {
    *harness(world).routing.hold_child.lock().unwrap() = Some(AgentUuid::new(child));
}

// ── When ─────────────────────────────────────────────────────────────────────

#[when("the fleet teardown runs")]
fn when_fleet_runs(world: &mut QuectoWorld) {
    let rt = runtime(world);
    let fleet = harness(world).fleet.clone();
    let outcome = bounded(&rt, fleet.execute(request()));
    state(world).outcomes.push(outcome);
    state(world).runtime = Some(rt);
}

#[when("the fleet teardown is triggered twice at once")]
fn when_fleet_twice(world: &mut QuectoWorld) {
    let rt = runtime(world);
    let fleet = harness(world).fleet.clone();
    let routing = harness(world).routing.clone();
    let outcomes = rt.block_on(async {
        let first = tokio::spawn({
            let fleet = fleet.clone();
            async move { fleet.execute(request()).await }
        });
        while routing.calls().len() < 2 {
            tokio::task::yield_now().await;
        }
        let second = tokio::spawn({
            let fleet = fleet.clone();
            async move { fleet.execute(request()).await }
        });
        // Both callers must be parked on the run before it is released,
        // otherwise the second could arrive after completion and start a
        // run of its own.
        while fleet.waiting_joiners() < 2 {
            tokio::task::yield_now().await;
        }
        routing.gate.notify_one();
        let first = tokio::time::timeout(Duration::from_secs(10), first)
            .await
            .unwrap()
            .unwrap();
        let second = tokio::time::timeout(Duration::from_secs(10), second)
            .await
            .unwrap()
            .unwrap();
        vec![first, second]
    });
    state(world).outcomes = outcomes;
    state(world).runtime = Some(rt);
}

#[when(expr = "the operator kill of child {string} finishes while the fleet teardown runs")]
fn when_kill_finishes(world: &mut QuectoWorld, child: String) {
    let rt = runtime(world);
    let fleet = harness(world).fleet.clone();
    let rows = harness(world).rows.clone();
    let routing = harness(world).routing.clone();
    let outcome = rt.block_on(async {
        let run = tokio::spawn({
            let fleet = fleet.clone();
            async move { fleet.execute(request()).await }
        });
        // The batch is polled in lineage order: once the sibling was asked,
        // the claimed child is already parked on its compensation.
        while routing.calls().is_empty() {
            tokio::task::yield_now().await;
        }
        let identity = rows.resolve(&child).unwrap();
        assert_eq!(
            rows.claim_terminal(&identity),
            quecto::application::subagents::ports::TerminalClaim::Claimed
        );
        // The kill's compensation ran.
        rows.set(&child, RowPhase::Compensated);
        tokio::time::timeout(Duration::from_secs(10), run)
            .await
            .unwrap()
            .unwrap()
    });
    state(world).outcomes.push(outcome);
    state(world).runtime = Some(rt);
}

#[when(expr = "the harness shutdown is admitted and executed for {string}")]
fn when_shutdown(world: &mut QuectoWorld, reason: String) {
    let rt = runtime(world);
    let reason = ShutdownReason::parse(&reason).unwrap();
    let harness = harness(world);
    let prepared = harness.prepared(reason);
    let execute = harness.execute.clone();
    let outcome = bounded(&rt, execute.execute(&prepared.token)).expect("shutdown outcome");
    state(world).shutdown = Some(outcome);
    state(world).runtime = Some(rt);
}

#[when("a spawn is registered against a frozen harness")]
fn when_spawn_frozen(world: &mut QuectoWorld) {
    use quecto::infrastructure::tools::harness_lifecycle::new_shared_harness_lifecycle;
    use quecto::infrastructure::tools::spawn::register_and_broadcast;
    use quecto::infrastructure::tools::subagent_registry::{SubagentEntry, new_registry};
    let registry = new_registry();
    let lifecycle = new_shared_harness_lifecycle();
    *lifecycle.lock().unwrap() = HarnessLifecycleState::Frozen;
    let refused = register_and_broadcast(
        &registry,
        None,
        "late",
        SubagentEntry::new("/tmp/late.sock".into(), 0),
        &lifecycle,
    )
    .expect_err("a frozen harness admits no child");
    assert!(registry.lock().unwrap().is_empty());
    state(world).spawn_refusal = Some(refused.to_string());
}

#[when("a spawn is registered against an accepting harness")]
fn when_spawn_accepting(world: &mut QuectoWorld) {
    use quecto::infrastructure::tools::harness_lifecycle::new_shared_harness_lifecycle;
    use quecto::infrastructure::tools::spawn::register_and_broadcast;
    use quecto::infrastructure::tools::subagent_registry::{SubagentEntry, new_registry};
    let registry = new_registry();
    register_and_broadcast(
        &registry,
        None,
        "early",
        SubagentEntry::new("/tmp/early.sock".into(), 0),
        &new_shared_harness_lifecycle(),
    )
    .expect("an accepting harness admits a child");
    assert_eq!(registry.lock().unwrap().len(), 1);
    state(world).spawn_refusal = None;
}

#[when(
    expr = "a busy client sends delete_all_subagents for {int} launched rows with unreachable sockets"
)]
fn when_busy_delete(world: &mut QuectoWorld, rows: usize) {
    use quecto::composition::subagent_teardown::{FleetTeardownWiring, build_fleet_teardown};
    use quecto::infrastructure::tools::subagent_registry::{SubagentEntry, new_registry};
    let registry = new_registry();
    for index in 0..rows {
        let uuid = AgentUuid::mint();
        let mut entry = SubagentEntry::with_identity(
            uuid.clone(),
            format!("busy-{index}"),
            std::path::PathBuf::from(format!("/nonexistent/busy-{index}.sock")),
            0,
        );
        entry.launch_generation = Some(quecto::domain::subagent_teardown::LaunchGeneration::new(
            index as u64 + 1,
        ));
        registry.lock().unwrap().insert(uuid.into_string(), entry);
    }
    let fleet = build_fleet_teardown(FleetTeardownWiring {
        owner: AgentUuid::new("root"),
        registry: registry.clone(),
        broadcast_tx: None,
        notify_tx: None,
        harness_lifecycle:
            quecto::infrastructure::tools::harness_lifecycle::new_shared_harness_lifecycle(),
    });
    let rt = runtime(world);
    let (handled, response) =
        rt.block_on(quecto::interface::cli::busy_reader_intercept_with_fleet(
            r#"{"type":"delete_all_subagents","id":"busy-del"}"#,
            registry.clone(),
            Some(fleet),
        ));
    assert!(handled, "served off the dispatch loop");
    state(world).busy_response = response;
    state(world).busy_registry = Some(registry);
    state(world).runtime = Some(rt);
}

// ── Then ─────────────────────────────────────────────────────────────────────

fn last_outcome(world: &mut QuectoWorld) -> FleetTeardownOutcome {
    state(world)
        .outcomes
        .last()
        .cloned()
        .expect("a fleet outcome")
        .expect("the run completed")
}

#[then(expr = "children {string} settled {string} and none is unsettled")]
fn then_settled(world: &mut QuectoWorld, children: String, result: String) {
    let outcome = last_outcome(world);
    let expected: Vec<&str> = children.split(',').map(str::trim).collect();
    let settled: Vec<&str> = outcome
        .settled
        .iter()
        .map(|settled| settled.child.uuid.as_str())
        .collect();
    assert_eq!(settled, expected, "{outcome:?}");
    for settled in &outcome.settled {
        assert_eq!(settled.result.as_str(), result, "{outcome:?}");
    }
    assert!(outcome.is_settled(), "{outcome:?}");
}

#[then("every direct child was claimed, asked once, concluded and compensated in that order")]
fn then_order(world: &mut QuectoWorld) {
    let harness = harness(world);
    let asked: Vec<String> = harness
        .routing
        .calls()
        .iter()
        .map(|call| format!("{call:?}"))
        .collect();
    assert_eq!(asked.len(), 2, "A and D asked once each: {asked:?}");
    assert!(asked.iter().all(|call| call.contains("Shutdown")));
    let concluded = harness.rows.concluded.lock().unwrap().len();
    assert_eq!(concluded, 2);
    let compensated = harness.rows.compensated.lock().unwrap().clone();
    assert_eq!(compensated.len(), 2);
    assert!(
        compensated
            .iter()
            .all(|(_, cause)| *cause == TerminationCause::OwnerTeardown)
    );
    // No row is left: the tombstones were pruned.
    assert!(harness.rows.phases.lock().unwrap().is_empty());
    // B and C, A's own children, were never addressed from here.
    assert!(
        !asked
            .iter()
            .any(|call| call.contains("\"B\"") || call.contains("\"C\""))
    );
}

#[then("the tombstones were pruned and the roster is empty")]
fn then_pruned(world: &mut QuectoWorld) {
    let outcome = last_outcome(world);
    assert_eq!(outcome.pruned.len(), outcome.settled.len());
    assert!(harness(world).rows.phases.lock().unwrap().is_empty());
}

#[then("both triggers observe one run and no child was asked twice")]
fn then_joined(world: &mut QuectoWorld) {
    let outcomes: Vec<FleetTeardownOutcome> = state(world)
        .outcomes
        .iter()
        .cloned()
        .map(|outcome| outcome.expect("completed"))
        .collect();
    assert_eq!(outcomes.len(), 2);
    assert_eq!(
        outcomes.iter().filter(|outcome| outcome.joined).count(),
        1,
        "{outcomes:?}"
    );
    assert_eq!(outcomes[0].settled, outcomes[1].settled);
    assert_eq!(harness(world).routing.calls().len(), 2);
    assert_eq!(harness(world).rows.compensated.lock().unwrap().len(), 2);
}

#[then(expr = "child {string} is reported unsettled with its claim lifted and {string} settled")]
fn then_unsettled(world: &mut QuectoWorld, unsettled: String, settled: String) {
    let outcome = last_outcome(world);
    assert!(!outcome.is_settled());
    let names: Vec<&str> = outcome
        .unsettled
        .iter()
        .map(|(uuid, _)| uuid.as_str())
        .collect();
    assert_eq!(names, [unsettled.as_str()], "{outcome:?}");
    let harness = harness(world);
    assert_eq!(harness.rows.phase(&unsettled), Some(RowPhase::Live));
    let _ = settled;
}

#[then(expr = "child {string} was joined, never asked, and {string} settled gracefully")]
fn then_joined_child(world: &mut QuectoWorld, joined: String, other: String) {
    let outcome = last_outcome(world);
    let results: Vec<(&str, FleetChildResult)> = outcome
        .settled
        .iter()
        .map(|settled| (settled.child.uuid.as_str(), settled.result))
        .collect();
    assert_eq!(
        results,
        [
            (joined.as_str(), FleetChildResult::Joined),
            (other.as_str(), FleetChildResult::Graceful)
        ],
        "{outcome:?}"
    );
    let asked: Vec<String> = harness(world)
        .routing
        .calls()
        .iter()
        .map(|call| format!("{call:?}"))
        .collect();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert!(asked[0].contains(&format!("\"{other}\"")));
}

#[then(
    "the shutdown cancelled the turn, settled the fleet, persisted and signalled exit in that order"
)]
fn then_shutdown_order(world: &mut QuectoWorld) {
    let outcome = state(world).shutdown.clone().expect("shutdown outcome");
    assert!(outcome.turn_cancelled);
    assert_eq!(
        outcome.children_shut_down,
        [AgentUuid::new("A"), AgentUuid::new("D")]
    );
    assert!(outcome.children_failed.is_empty());
    assert!(outcome.exit_signalled);
    let harness = harness(world);
    assert_eq!(harness.cancellation.calls.load(Ordering::SeqCst), 1);
    assert_eq!(harness.persistence.calls.lock().unwrap().len(), 1);
    assert_eq!(harness.exit.signalled().len(), 1);
    assert_eq!(
        harness.lifecycle.lifecycle(),
        HarnessLifecycleState::Terminated
    );
    assert!(harness.rows.phases.lock().unwrap().is_empty());
}

#[then("the shutdown reports the survivor as failed and still completes")]
fn then_shutdown_survivor(world: &mut QuectoWorld) {
    let outcome = state(world).shutdown.clone().expect("shutdown outcome");
    assert!(!outcome.children_failed.is_empty(), "{outcome:?}");
    assert!(outcome.exit_signalled);
    assert_eq!(
        harness(world).lifecycle.lifecycle(),
        HarnessLifecycleState::Terminated
    );
}

#[then("the spawn is refused because the harness is frozen")]
fn then_spawn_refused(world: &mut QuectoWorld) {
    let refusal = state(world).spawn_refusal.clone().expect("refused");
    assert!(refusal.contains("spawn refused"), "{refusal}");
    assert!(refusal.contains("Frozen"), "{refusal}");
}

#[then("the spawn is admitted")]
fn then_spawn_admitted(world: &mut QuectoWorld) {
    assert!(state(world).spawn_refusal.is_none());
}

#[then(
    expr = "the busy delete response reports {int} removed as unobserved and the registry is empty"
)]
fn then_busy_delete(world: &mut QuectoWorld, removed: u64) {
    let response = state(world).busy_response.clone().expect("a response line");
    assert_eq!(response["command"], "delete_all_subagents");
    assert_eq!(response["id"], "busy-del");
    assert_eq!(response["success"], true, "{response}");
    assert_eq!(response["data"]["removed"], removed, "{response}");
    for settled in response["data"]["settled"].as_array().unwrap() {
        assert_eq!(settled["result"], "unobserved", "{settled}");
    }
    let registry = state(world).busy_registry.clone().unwrap();
    assert!(registry.lock().unwrap().is_empty());
}

#[then("the fleet run is reported interrupted and the next run starts afresh")]
fn then_interrupted(world: &mut QuectoWorld) {
    let rt = runtime(world);
    let harness = harness(world);
    // Build a fleet whose spawner drops the first run.
    let spawner = Arc::new(crate::common::teardown_fixture::Spawner::default());
    spawner.drop_next.store(1, Ordering::SeqCst);
    let fleet = quecto::application::subagents::use_cases::TerminateAllDelegatedAgents::new(
        quecto::application::subagents::use_cases::TerminateAllDelegatedAgentsPorts {
            lifecycle: harness.lifecycle.clone(),
            registry: harness.rows.clone(),
            routing: harness.routing.clone(),
            termination: harness.rows.clone(),
            compensation: harness.rows.clone(),
            spawner,
        },
    );
    let first = bounded(&rt, fleet.execute(request()));
    assert_eq!(first, Err(FleetTeardownError::Interrupted));
    assert!(!fleet.in_flight());
    assert_eq!(harness.rows.phase("A"), Some(RowPhase::Live));
    let second = bounded(&rt, fleet.execute(request())).expect("a fresh run");
    assert!(!second.joined);
    assert_eq!(second.settled.len(), 2);
    state(world).runtime = Some(rt);
}
