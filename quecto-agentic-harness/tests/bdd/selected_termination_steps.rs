//! BDD steps for operator-selected termination (#1936, #1882), over the
//! fixture in `selected_termination_fixture.rs`.
use std::path::PathBuf;
use std::time::Duration;

use cucumber::{given, then, when};
use quecto::application::subagents::dto::{
    KillDelegatedAgentError, ObserveOwnedChildExitRequest, ObservedExit, TerminationResult,
};
use quecto::application::subagents::ports::{
    ExitObservation, ResolutionError, TerminationConclusion,
};
use quecto::domain::ids::AgentUuid;
use quecto::domain::subagent_teardown::{
    DelegatedAgentIdentity, LaunchGeneration, TerminationRouteError,
};
use quecto::infrastructure::tools::subagent_registry::{
    SubagentEntry, SubagentStatus, TeardownPhase,
};

use crate::QuectoWorld;

#[path = "selected_termination_fixture.rs"]
pub(crate) mod fixture;
pub(crate) use fixture::SelectedTerminationState;
use fixture::{
    Behaviour, Process, add_launched, add_reported, behaviour_of, body, broadcasts, label, listed,
    rollback, state, wait_compensated,
};

// ── Given ──────────────────────────────────────────────────────────────────

#[given(expr = "a root harness whose launched child {string} {word} commands")]
fn given_root_child(world: &mut QuectoWorld, uuid: String, verb: String) {
    add_launched(
        world,
        &uuid,
        behaviour_of(&format!("{verb} commands")),
        Process::None,
    );
}

#[given(expr = "a root harness whose launched child {string} is unreachable")]
fn given_root_child_unreachable(world: &mut QuectoWorld, uuid: String) {
    add_launched(world, &uuid, Behaviour::Unreachable, Process::None);
}

#[given(
    expr = "a root harness whose launched child {string} acknowledges commands and exits when told"
)]
fn given_root_child_exits(world: &mut QuectoWorld, uuid: String) {
    add_launched(world, &uuid, Behaviour::Acknowledge, Process::ExitsWhenTold);
}

#[given(
    expr = "a root harness whose launched child {string} acknowledges commands and holds a short-lived process"
)]
fn given_root_child_short(world: &mut QuectoWorld, uuid: String) {
    add_launched(world, &uuid, Behaviour::Acknowledge, Process::ShortLived);
}

#[given(expr = "a root harness whose launched child {string} {} and holds a sleeping process")]
fn given_root_child_behaviour_sleeping(world: &mut QuectoWorld, uuid: String, behaviour: String) {
    add_launched(world, &uuid, behaviour_of(&behaviour), Process::Sleeping);
}

#[given(expr = "{string} reported descendants {string} and {string} with launch generations")]
fn given_reported(world: &mut QuectoWorld, parent: String, b: String, c: String) {
    add_reported(world, &b, &parent, 1);
    add_reported(world, &c, &parent, 1);
}

#[given(expr = "a launched child {string} that acknowledges commands")]
fn given_launched_ack(world: &mut QuectoWorld, uuid: String) {
    add_launched(world, &uuid, Behaviour::Acknowledge, Process::None);
}

#[given("nothing else")]
fn given_nothing(_world: &mut QuectoWorld) {}

#[given(expr = "a second live row also labelled {string}")]
fn given_duplicate_label(world: &mut QuectoWorld, label_text: String) {
    let s = state(world);
    let mut entry = SubagentEntry::with_identity(
        AgentUuid::new("A2"),
        label_text,
        PathBuf::from("/tmp/a2.sock"),
        0,
    );
    entry.launch_generation = Some(LaunchGeneration::new(99));
    s.registry
        .as_ref()
        .unwrap()
        .lock()
        .unwrap()
        .insert("A2".into(), entry);
}

#[given(expr = "{string} has already been compensated")]
fn given_compensated(world: &mut QuectoWorld, uuid: String) {
    let s = state(world);
    s.registry.as_ref().unwrap().lock().unwrap()[&uuid]
        .teardown
        .send_replace(TeardownPhase::Compensated);
}

#[given(expr = "a fixture row {string} without a launch generation")]
fn given_fixture(world: &mut QuectoWorld, uuid: String) {
    let s = state(world);
    s.registry.as_ref().unwrap().lock().unwrap().insert(
        uuid.clone(),
        SubagentEntry::with_identity(
            AgentUuid::new(&uuid),
            label(&uuid),
            PathBuf::from("/tmp/f.sock"),
            0,
        ),
    );
}

#[given(expr = "reported rows {string} and {string} whose parents form a cycle")]
fn given_cycle(world: &mut QuectoWorld, y: String, z: String) {
    add_reported(world, &y, &z, 1);
    add_reported(world, &z, &y, 1);
}

#[given(expr = "a termination of {string} already in flight")]
fn given_in_flight(world: &mut QuectoWorld, uuid: String) {
    let s = state(world);
    s.registry.as_ref().unwrap().lock().unwrap()[&uuid]
        .teardown
        .send_replace(TeardownPhase::Stopping(
            quecto::infrastructure::tools::subagent_registry::StoppingClaim::first(
                quecto::infrastructure::tools::subagent_registry::TeardownIntent::SelectedTermination,
            ),
        ));
}

#[given(expr = "the reaper of {string} is running")]
fn given_reaper(world: &mut QuectoWorld, uuid: String) {
    let s = state(world);
    let handle = s
        .handles
        .iter()
        .find(|(id, _)| id == &uuid)
        .map(|(_, handle)| *handle)
        .expect("an owned process");
    let registry = s.registry.clone().unwrap();
    let supervisor = s.supervisor.clone().unwrap();
    let observer = s.lifecycle.clone().unwrap().observe_exit;
    let child = registry.lock().unwrap()[&uuid]
        .delegated_identity()
        .unwrap();
    let exit_tx = registry.lock().unwrap()[&uuid]
        .exit_signal_tx
        .clone()
        .unwrap();
    // The production reaper: it publishes the exit, hands it to the
    // observation use case and retires the handle.
    let _guard = s.runtime.as_ref().unwrap().enter();
    quecto::infrastructure::tools::spawn_reaper::spawn_reaper_task(
        handle,
        supervisor,
        quecto::infrastructure::tools::spawn_reaper::ReaperContext {
            exit_tx,
            child,
            observer,
            swarm_member: None,
        },
    );
}

#[given("an AgentCmdTool over the root registry with the composed kill owner")]
fn given_agent_cmd_over_root(world: &mut QuectoWorld) {
    let (registry, kill) = {
        let s = state(world);
        (s.registry.clone().unwrap(), s.kill.clone().unwrap())
    };
    world.agent_cmd_registry = Some(registry.clone());
    world.agent_cmd_tool = Some(
        quecto::infrastructure::tools::agent_cmd::AgentCmdTool::new(registry).with_kill_tool(kill),
    );
}

// ── When ───────────────────────────────────────────────────────────────────

fn run_kill(world: &mut QuectoWorld, reference: &str) {
    let s = state(world);
    let kill = s.kill.clone().unwrap();
    let arguments = serde_json::json!({"agent_id": reference, "command": "kill"}).to_string();
    let result = s.runtime.as_ref().unwrap().block_on(async move {
        tokio::time::timeout(Duration::from_secs(30), kill.execute(&arguments))
            .await
            .expect("kill settles within the bound")
            .unwrap()
    });
    s.result = Some(result);
}

#[when(expr = "the operator kills {string}")]
fn when_kill(world: &mut QuectoWorld, reference: String) {
    run_kill(world, &reference);
}

#[when(expr = "the operator kills {string} and {string} then reports a snapshot without {string}")]
fn when_kill_nested(world: &mut QuectoWorld, reference: String, via: String, pruned: String) {
    let s = state(world);
    let registry = s.registry.clone().unwrap();
    let broadcast_tx = s.broadcast_tx.clone().unwrap();
    // A's next snapshot arrives while the kill waits: it omits the pruned
    // row, and the merge (the production monitor path) compensates it.
    let reporter = s.runtime.as_ref().unwrap().spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        let survivors: Vec<serde_json::Value> = {
            let entries = registry.lock().unwrap();
            entries
                .iter()
                .filter(|(key, entry)| {
                    entry.parent_id.as_deref() == Some(via.as_str()) && *key != &pruned
                })
                .map(|(key, entry)| {
                    serde_json::json!({
                        "agentId": entry.display_name, "agentUuid": key, "status": "idle",
                        "pid": entry.pid, "parentId": via, "executionBackend": "local",
                        "launchGeneration": entry.reported_generation.map(|g| g.get()),
                    })
                })
                .collect()
        };
        let line = serde_json::json!({"type": "subagent_state_changed", "subagents": survivors})
            .to_string();
        let forwarded =
            quecto::infrastructure::tools::subagent_monitor::forward_child_state_changed(
                &line, &registry, &via,
            )
            .expect("a state_changed line is merged");
        let _ = broadcast_tx.send(forwarded);
    });
    run_kill(world, &reference);
    let s = state(world);
    s.runtime.as_ref().unwrap().block_on(reporter).unwrap();
}

#[when(expr = "the monitor of {string} observes its connection closed")]
fn when_monitor_closed(world: &mut QuectoWorld, uuid: String) {
    let s = state(world);
    let observer = s.lifecycle.clone().unwrap().observe_exit;
    let child = s.registry.as_ref().unwrap().lock().unwrap()[&uuid]
        .delegated_identity()
        .unwrap();
    let observed =
        s.runtime
            .as_ref()
            .unwrap()
            .block_on(observer.execute(ObserveOwnedChildExitRequest {
                child,
                observation: ExitObservation::ConnectionClosed,
            }));
    s.observed = Some(observed);
}

#[when(expr = "the process of {string} ends")]
fn when_process_ends(world: &mut QuectoWorld, uuid: String) {
    let s = state(world);
    let pid = s.registry.as_ref().unwrap().lock().unwrap()[&uuid].pid;
    // SAFETY: the pid is this scenario's own sleeping fixture.
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
    wait_compensated(s, &uuid);
}

#[when(expr = "the registered launch of {string} is rolled back")]
fn when_rollback(world: &mut QuectoWorld, uuid: String) {
    rollback(world, &uuid);
}

#[when(expr = "the registered launch of {string} is rolled back twice")]
fn when_rollback_twice(world: &mut QuectoWorld, uuid: String) {
    rollback(world, &uuid);
    rollback(world, &uuid);
}

#[when(expr = "{string} forwards a snapshot listing {string} with launch generation {int}")]
fn when_forwards(world: &mut QuectoWorld, via: String, child: String, generation: u64) {
    let s = state(world);
    let registry = s.registry.clone().unwrap();
    let line = serde_json::json!({"type": "subagent_state_changed", "subagents": [{
        "agentId": label(&child), "agentUuid": child, "status": "idle", "pid": 4242,
        "parentId": via, "executionBackend": "local", "launchGeneration": generation,
    }]})
    .to_string();
    let forwarded = quecto::infrastructure::tools::subagent_monitor::forward_child_state_changed(
        &line, &registry, &via,
    )
    .expect("merged");
    assert!(forwarded.contains("\"launchGeneration\""));
}

// ── Then ───────────────────────────────────────────────────────────────────

#[then(expr = "the kill result is {string} and removed {string}")]
fn then_result_removed_one(world: &mut QuectoWorld, result: String, removed: String) {
    let (target, killed) = killed_by(world, &result);
    assert_eq!(target, removed, "the target comes first");
    assert_eq!(killed, vec![removed]);
}

#[then(expr = "the kill result is {string} and removed {string}, {string} and {string}")]
fn then_result_removed_three(
    world: &mut QuectoWorld,
    result: String,
    a: String,
    b: String,
    c: String,
) {
    let (target, killed) = killed_by(world, &result);
    assert_eq!(target, a, "the target comes first");
    let mut expected = vec![a, b, c];
    expected.sort();
    assert_eq!(killed, expected);
}

/// The last kill succeeded with `result`: its target and the sorted list
/// of every uuid it reported removed.
fn killed_by(world: &mut QuectoWorld, result: &str) -> (String, Vec<String>) {
    let is_error = state(world).result.as_ref().unwrap().is_error;
    let body = body(world);
    assert!(!is_error, "{body}");
    assert_eq!(body["result"], result, "{body}");
    let mut killed: Vec<String> = body["killed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    let target = killed.first().cloned().expect("a removed target");
    killed.sort();
    (target, killed)
}

#[then(expr = "the kill result is {string}")]
fn then_result(world: &mut QuectoWorld, result: String) {
    let is_error = state(world).result.as_ref().unwrap().is_error;
    let body = body(world);
    assert_eq!(body["result"], result, "{body}");
    assert_eq!(is_error, result == "failed");
}

/// #1953 review (1): a claim kept after effects is not a dead end. The
/// retry is not refused as in flight — nothing is executing — but re-takes
/// the claim (attempt 2) and re-attempts the protocol. The retry's own
/// wait is not awaited here (it would run the compensation bound again):
/// the second `shutdown` reaching the child is the observation.
#[then(expr = "a retry of the kill of {string} re-takes the claim and sends a second shutdown")]
fn then_retry_re_attempts(world: &mut QuectoWorld, reference: String) {
    use quecto::infrastructure::tools::subagent_registry::{ClaimOwner, TeardownPhase};
    let s = state(world);
    let requests = s
        .endpoints
        .iter()
        .find(|(id, _)| id == &reference)
        .map(|(_, endpoint)| endpoint.requests.clone())
        .expect("the child's endpoint");
    let shutdowns = |requests: &std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>| {
        requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r["type"] == "shutdown")
            .count()
    };
    assert_eq!(shutdowns(&requests), 1, "the failed kill sent one shutdown");
    let kill = s.kill.clone().unwrap();
    let arguments = serde_json::json!({"agent_id": reference, "command": "kill"}).to_string();
    // Detached: the retry waits its own bound; the scenario observes the
    // re-taken claim and the second command, not its eventual answer.
    let _retry = s
        .runtime
        .as_ref()
        .unwrap()
        .spawn(async move { kill.execute(&arguments).await });
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while shutdowns(&requests) < 2 {
        assert!(
            std::time::Instant::now() < deadline,
            "the retry never sent a second shutdown"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let entries = s.registry.as_ref().unwrap().lock().unwrap();
    match entries[&reference].teardown_phase() {
        TeardownPhase::Stopping(claim) => {
            assert_eq!(claim.attempt, 2, "re-taken, not refused");
            assert_eq!(claim.owner, ClaimOwner::Executing);
        }
        other => panic!("the retry holds the claim: {other:?}"),
    }
}

#[then(expr = "the kill of {string} is refused with {string}")]
fn then_retry_refused(world: &mut QuectoWorld, reference: String, detail: String) {
    run_kill(world, &reference);
    let s = state(world);
    let result = s.result.as_ref().unwrap();
    assert!(result.is_error, "{}", result.content);
    assert!(
        result.content.contains(&detail),
        "{:?} lacks {detail:?}",
        result.content
    );
}

#[then(expr = "the kill is refused with {string}")]
fn then_refused(world: &mut QuectoWorld, detail: String) {
    let s = state(world);
    let result = s.result.as_ref().unwrap();
    assert!(result.is_error, "{}", result.content);
    assert!(
        result.content.contains(&detail),
        "{:?} lacks {detail:?}",
        result.content
    );
}

#[then(
    expr = "{string} received exactly one terminate_delegated_agent command for {string} and no shutdown"
)]
fn then_forwarded(world: &mut QuectoWorld, via: String, target: String) {
    let s = state(world);
    let requests = s
        .endpoints
        .iter()
        .find(|(id, _)| id == &via)
        .map(|(_, endpoint)| endpoint.requests.lock().unwrap().clone())
        .unwrap();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0]["type"], "terminate_delegated_agent");
    assert_eq!(requests[0]["target_uuid"], target);
}

#[then(expr = "no command reached {string}")]
fn then_no_command(world: &mut QuectoWorld, uuid: String) {
    let s = state(world);
    let requests = s
        .endpoints
        .iter()
        .find(|(id, _)| id == &uuid)
        .map(|(_, endpoint)| endpoint.requests.lock().unwrap().clone())
        .unwrap();
    assert!(requests.is_empty(), "{requests:?}");
}

/// The rows among `uuids` that are no longer live: exited, or past the
/// `Live` teardown phase.
fn not_live(world: &mut QuectoWorld, uuids: &[&str]) -> Vec<String> {
    let s = state(world);
    let entries = s.registry.as_ref().unwrap().lock().unwrap();
    uuids
        .iter()
        .filter(|uuid| {
            let entry = &entries[**uuid];
            entry.status == SubagentStatus::Exited || entry.teardown_phase() != TeardownPhase::Live
        })
        .map(|uuid| (*uuid).to_owned())
        .collect()
}

#[then(expr = "{string}, {string} and {string} remain live")]
fn then_three_live(world: &mut QuectoWorld, a: String, b: String, c: String) {
    let gone = not_live(world, &[&a, &b, &c]);
    assert!(gone.is_empty(), "not live: {gone:?}");
}

#[then(expr = "{string} and {string} remain live")]
fn then_two_live(world: &mut QuectoWorld, a: String, b: String) {
    let gone = not_live(world, &[&a, &b]);
    assert!(gone.is_empty(), "not live: {gone:?}");
}

#[then(expr = "{string} remains live")]
fn then_one_live(world: &mut QuectoWorld, a: String) {
    let gone = not_live(world, &[&a]);
    assert!(gone.is_empty(), "not live: {gone:?}");
}

#[then(expr = "{string} stays claimed stopping")]
fn then_claimed_stopping(world: &mut QuectoWorld, a: String) {
    let s = state(world);
    let entries = s.registry.as_ref().unwrap().lock().unwrap();
    assert_ne!(entries[&a].status, SubagentStatus::Exited);
    // Kept for the exit that will follow, and recorded as returned so a
    // later trigger may re-take it (#1936 review).
    assert_eq!(
        entries[&a].teardown_phase(),
        TeardownPhase::Stopping(
            quecto::infrastructure::tools::subagent_registry::StoppingClaim {
                intent: quecto::infrastructure::tools::subagent_registry::TeardownIntent::SelectedTermination,
                attempt: 1,
                owner: quecto::infrastructure::tools::subagent_registry::ClaimOwner::Returned,
            }
        ),
        "effects reached the child: its eventual exit is this kill's"
    );
}

#[then(expr = "{string} is not exited")]
fn then_not_exited(world: &mut QuectoWorld, a: String) {
    let s = state(world);
    let entries = s.registry.as_ref().unwrap().lock().unwrap();
    assert_ne!(entries[&a].status, SubagentStatus::Exited);
    assert!(!matches!(
        entries[&a].teardown_phase(),
        TeardownPhase::Compensating(_) | TeardownPhase::Compensated
    ));
}
#[then(
    expr = "exactly one survivor broadcast went out, listing {string}, {string} and {string} only"
)]
fn then_one_broadcast_three(world: &mut QuectoWorld, a: String, b: String, c: String) {
    let events = broadcasts(world);
    assert_eq!(events.len(), 1, "{events:?}");
    let mut expected = vec![a, b, c];
    expected.sort();
    assert_eq!(listed(&events[0]), expected);
}

#[then(expr = "exactly one survivor broadcast went out, listing {string} only")]
fn then_one_broadcast_one(world: &mut QuectoWorld, a: String) {
    let events = broadcasts(world);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(listed(&events[0]), vec![a]);
}

#[then("exactly one survivor broadcast went out, listing nothing")]
fn then_one_broadcast_empty(world: &mut QuectoWorld) {
    let events = broadcasts(world);
    assert_eq!(events.len(), 1, "{events:?}");
    assert!(listed(&events[0]).is_empty());
}

#[then("no survivor broadcast went out")]
fn then_no_broadcast(world: &mut QuectoWorld) {
    let events = broadcasts(world);
    assert!(events.is_empty(), "{events:?}");
}

#[then(expr = "the owned process of {string} was never signalled")]
fn then_never_signalled(world: &mut QuectoWorld, uuid: String) {
    let s = state(world);
    let handle = s.handles.iter().find(|(id, _)| id == &uuid).unwrap().1;
    let supervisor = s.supervisor.as_ref().unwrap();
    assert!(supervisor.signals_sent(handle).is_empty());
    assert!(supervisor.fallback_authorised_by(handle).is_none());
}

#[then(expr = "the owned process of {string} was signalled after the protocol outcome {string}")]
fn then_signalled_after(world: &mut QuectoWorld, uuid: String, negative: String) {
    let s = state(world);
    let handle = s.handles.iter().find(|(id, _)| id == &uuid).unwrap().1;
    let supervisor = s.supervisor.as_ref().unwrap();
    assert!(!supervisor.signals_sent(handle).is_empty());
    let why = supervisor.fallback_authorised_by(handle).unwrap();
    assert!(why.contains(&negative), "{why:?} lacks {negative:?}");
}

#[then(expr = "the row of {string} is compensated once")]
fn then_compensated_once(world: &mut QuectoWorld, uuid: String) {
    let s = state(world);
    wait_compensated(s, &uuid);
    let entries = s.registry.as_ref().unwrap().lock().unwrap();
    assert_eq!(entries[&uuid].status, SubagentStatus::Exited);
}

/// The kept claim is honoured: the exit the monitor observed later ran the
/// row's compensation as the kill (its exit signal is `Terminated`, not
/// the connection-level `ConnectionClosed` a natural EOF would carry), and
/// the row is compensated exactly once.
#[then(expr = "the observation compensated {string} as a selected termination, not a natural exit")]
fn then_compensated_as_kill(world: &mut QuectoWorld, uuid: String) {
    let s = state(world);
    assert!(
        matches!(s.observed, Some(ObservedExit::Compensated { .. })),
        "the observation ran the compensation: {:?}",
        s.observed
    );
    wait_compensated(s, &uuid);
    let entries = s.registry.as_ref().unwrap().lock().unwrap();
    let entry = &entries[&uuid];
    assert_eq!(entry.status, SubagentStatus::Exited);
    assert_eq!(entry.teardown_phase(), TeardownPhase::Compensated);
    let signal = entry
        .exit_signal_tx
        .as_ref()
        .expect("the launched row carries an exit signal")
        .borrow()
        .clone()
        .expect("the compensation published the exit");
    assert_eq!(
        signal.kind,
        quecto::infrastructure::tools::subagent_registry::ExitSignalKind::Terminated,
        "a kept claim makes the later exit this kill's, never a post-mortem"
    );
}

#[then(expr = "the observation is deferred to the process exit and {string} remains live")]
fn then_deferred(world: &mut QuectoWorld, uuid: String) {
    {
        let s = state(world);
        assert_eq!(s.observed, Some(ObservedExit::DeferredToProcessExit));
    }
    let gone = not_live(world, &[&uuid]);
    assert!(gone.is_empty(), "not live: {gone:?}");
}

#[then(expr = "the rollback concluded {string} and removed {string}")]
fn then_rollback(world: &mut QuectoWorld, conclusion: String, removed: String) {
    let s = state(world);
    let outcome = s.rollbacks.last().unwrap();
    assert_eq!(format!("{:?}", outcome.conclusion), conclusion);
    assert_eq!(outcome.removed, [AgentUuid::new(removed)]);
}

#[then(expr = "the second rollback concluded {string} and removed nothing")]
fn then_second_rollback(world: &mut QuectoWorld, conclusion: String) {
    let s = state(world);
    assert_eq!(s.rollbacks.len(), 2);
    let outcome = &s.rollbacks[1];
    assert_eq!(format!("{:?}", outcome.conclusion), conclusion);
    assert!(outcome.removed.is_empty());
}

#[then(expr = "the root's lineage lists {string} at generation {int} beneath {string}")]
fn then_lineage(world: &mut QuectoWorld, child: String, generation: u64, parent: String) {
    use quecto::application::subagents::ports::SubagentLifecycleRepository;
    let s = state(world);
    let lifecycle = quecto::composition::subagent_teardown::LifecycleAdapter::new(
        s.registry.clone(),
        AgentUuid::new("root"),
        quecto::infrastructure::tools::harness_lifecycle::new_shared_harness_lifecycle(),
    );
    let lineage = lifecycle.lineage();
    assert!(
        lineage.records.iter().any(|record| {
            record.identity
                == DelegatedAgentIdentity::new(child.as_str(), LaunchGeneration::new(generation))
                && record.parent == AgentUuid::new(&parent)
        }),
        "{lineage:?}"
    );
}

#[then(expr = "the root's own snapshot carries launch generations for {string} and {string}")]
fn then_snapshot_generations(world: &mut QuectoWorld, a: String, b: String) {
    let s = state(world);
    let event: serde_json::Value = serde_json::from_str(
        quecto::infrastructure::tools::subagent_cascade::build_state_changed_event(
            s.registry.as_ref().unwrap(),
        )
        .trim(),
    )
    .unwrap();
    for uuid in [a, b] {
        let row = event["subagents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["agentUuid"] == uuid)
            .unwrap();
        assert!(row["launchGeneration"].is_u64(), "{row}");
    }
}

#[then(expr = "a kill of {string} is forwarded to {string} at generation {int}")]
fn then_kill_forwarded(world: &mut QuectoWorld, child: String, via: String, generation: u64) {
    // The ancestor's next snapshot omits the child (the merge prune marks
    // its row compensated), which is what the forwarded kill observes.
    let s = state(world);
    let entries = s.registry.clone().unwrap();
    let pruner = {
        let child = child.clone();
        s.runtime.as_ref().unwrap().spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let mut guard = entries.lock().unwrap();
            let next =
                quecto::infrastructure::tools::subagent_cascade::next_roster_sequence(&guard);
            let row = guard.get_mut(&child).unwrap();
            quecto::infrastructure::tools::subagent_cascade::mark_entry_dead(row, next);
            quecto::infrastructure::tools::subagent_cascade::mark_entry_compensated(row);
        })
    };
    run_kill(world, &child);
    let s = state(world);
    s.runtime.as_ref().unwrap().block_on(pruner).unwrap();
    let requests = s
        .endpoints
        .iter()
        .find(|(id, _)| id == &via)
        .map(|(_, endpoint)| endpoint.requests.lock().unwrap().clone())
        .unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["type"], "terminate_delegated_agent");
    assert_eq!(requests[0]["target_generation"], generation);
    let body = body(world);
    assert_eq!(body["result"], "graceful", "{body}");
}

#[then("every termination result and kill error renders its vocabulary")]
fn then_vocabulary(_world: &mut QuectoWorld) {
    assert_eq!(TerminationResult::Graceful.as_str(), "graceful");
    assert_eq!(TerminationResult::Fallback.to_string(), "fallback");
    assert_eq!(TerminationResult::AlreadyExited.as_str(), "already-exited");
    for (error, expected) in [
        (
            KillDelegatedAgentError::Unresolved(ResolutionError::NotDelegated),
            "not a delegated agent",
        ),
        (
            KillDelegatedAgentError::AlreadyStopping,
            "already in flight",
        ),
        (
            KillDelegatedAgentError::Rejected(TerminationRouteError::TargetIsSelf),
            "rejected",
        ),
        (KillDelegatedAgentError::NotAccepting, "not accepting"),
        (
            KillDelegatedAgentError::RouteUnreachable {
                via: AgentUuid::new("A"),
                detail: "gone".into(),
            },
            "route via A unreachable",
        ),
        (
            KillDelegatedAgentError::Failed {
                detail: "x".into(),
                effects_dispatched: false,
            },
            "termination failed",
        ),
    ] {
        assert!(error.to_string().contains(expected), "{error}");
    }
    for conclusion in [
        TerminationConclusion::NoRetainedHandle,
        TerminationConclusion::StillRunning("x".into()),
    ] {
        assert!(!format!("{conclusion:?}").is_empty());
    }
    for error in [
        quecto::application::subagents::ports::StoppingClaimError::Unknown,
        quecto::application::subagents::ports::StoppingClaimError::AlreadyStopping,
        quecto::application::subagents::ports::StoppingClaimError::Exited,
    ] {
        assert!(!error.to_string().is_empty());
    }
    for error in [
        ResolutionError::Unknown,
        ResolutionError::Ambiguous,
        ResolutionError::Exited,
    ] {
        assert!(!error.to_string().is_empty());
    }
}
