//! Steps for `uds_subagent_liveness.feature` — child progress must keep
//! flowing while the parent turn occupies the serial dispatch loop
//! (the child-progress-freeze fix, 2026-07-29).

use super::*;
use quecto::domain::agent::AgentProgressEvent;
use quecto::domain::message::Message;

fn inner_turn() -> AgentProgressEvent {
    AgentProgressEvent::TurnCompleted {
        messages: vec![
            Message::user("question"),
            Message::assistant("inner-turn answer", vec![]),
        ]
        .into(),
    }
}

fn ledger_hints(lines: &[serde_json::Value]) -> Vec<&serde_json::Value> {
    lines
        .iter()
        .filter(|l| l["type"] == "ledger_advanced")
        .collect()
}

fn run_turn_events(world: &mut QuectoWorld, events: &[AgentProgressEvent]) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    world.subagent_liveness_lines =
        Some(rt.block_on(cli::ledger_hint_lines_for_turn_events(events)));
}

fn run_busy_intercept(world: &mut QuectoWorld, line: &str) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let (handled, response) = rt.block_on(cli::busy_reader_intercept(line));
    world.subagent_liveness_intercept = Some((handled, response));
}

#[given("a prompt turn is in flight against a fresh conversation snapshot")]
fn fresh_snapshot(world: &mut QuectoWorld) {
    world.subagent_liveness_lines = None;
}

#[when("an inner turn completes with new messages")]
fn inner_turn_completes(world: &mut QuectoWorld) {
    run_turn_events(world, &[inner_turn()]);
}

#[then("a ledger advance hint should be emitted before the prompt finishes")]
fn hint_emitted(world: &mut QuectoWorld) {
    let lines = world.subagent_liveness_lines.as_ref().expect("event lines");
    assert!(
        !ledger_hints(lines).is_empty(),
        "mid-turn TurnCompleted must emit ledger_advanced, got: {lines:?}"
    );
}

#[when("the same inner turn completes twice")]
fn same_turn_twice(world: &mut QuectoWorld) {
    let event = inner_turn();
    run_turn_events(world, &[event.clone(), event]);
}

#[then("exactly one ledger advance hint should be emitted")]
fn exactly_one_hint(world: &mut QuectoWorld) {
    let lines = world.subagent_liveness_lines.as_ref().expect("event lines");
    assert_eq!(
        ledger_hints(lines).len(),
        1,
        "an unchanged republish must not spam hints, got: {lines:?}"
    );
}

#[when("only streaming tokens arrive")]
fn only_tokens(world: &mut QuectoWorld) {
    run_turn_events(
        world,
        &[
            AgentProgressEvent::Token("to".into()),
            AgentProgressEvent::Token("ken".into()),
        ],
    );
}

#[then("no ledger advance hint should be emitted")]
fn no_hint(world: &mut QuectoWorld) {
    let lines = world.subagent_liveness_lines.as_ref().expect("event lines");
    assert!(
        ledger_hints(lines).is_empty(),
        "tokens must not advance the ledger"
    );
}

#[given("the parent dispatch loop is occupied by a turn")]
fn dispatch_loop_busy(world: &mut QuectoWorld) {
    // The interceptor runs on the reader task and never touches the dispatch
    // loop, so "busy" needs no simulation: the assertion is that the command
    // is fully handled without ever reaching the dispatch channel.
    world.subagent_liveness_intercept = None;
}

#[when(expr = "a client sends get_subagents with correlation id {string}")]
fn send_get_subagents(world: &mut QuectoWorld, id: String) {
    run_busy_intercept(world, &format!(r#"{{"type":"get_subagents","id":"{id}"}}"#));
}

#[then("the command should be handled off the dispatch loop")]
fn handled_off_loop(world: &mut QuectoWorld) {
    let (handled, _) = world
        .subagent_liveness_intercept
        .as_ref()
        .expect("intercept result");
    assert!(
        handled,
        "liveness command must not queue behind the busy dispatch loop"
    );
}

#[then(expr = "the response should carry correlation id {string} and a snapshot marker")]
fn response_correlated(world: &mut QuectoWorld, id: String) {
    let (_, response) = world
        .subagent_liveness_intercept
        .as_ref()
        .expect("intercept result");
    let response = response.as_ref().expect("a response line");
    assert_eq!(response["command"], "get_subagents");
    assert_eq!(response["id"], id.as_str());
    assert_eq!(response["success"], true);
    assert_eq!(response["data"]["snapshot"], true);
}

#[when(expr = "a client sends a sync addressed to child {string}")]
fn send_child_sync(world: &mut QuectoWorld, child: String) {
    run_busy_intercept(
        world,
        &format!(r#"{{"type":"sync","id":"cs-live","agent_id":"{child}","epoch":0,"sinceRev":0}}"#),
    );
}

#[then("the client should receive a correlated sync response")]
fn sync_response_correlated(world: &mut QuectoWorld) {
    let (_, response) = world
        .subagent_liveness_intercept
        .as_ref()
        .expect("intercept result");
    let response = response.as_ref().expect("a response line");
    assert_eq!(response["command"], "sync");
    assert_eq!(response["id"], "cs-live");
}

#[when("a client sends a sync without a child address")]
fn send_parent_sync(world: &mut QuectoWorld) {
    run_busy_intercept(
        world,
        r#"{"type":"sync","id":"ps-live","epoch":0,"sinceRev":0}"#,
    );
}

#[then("the liveness interceptor should leave the command alone")]
fn left_alone(world: &mut QuectoWorld) {
    let (handled, response) = world
        .subagent_liveness_intercept
        .as_ref()
        .expect("intercept result");
    assert!(
        !handled,
        "a parent-scoped sync belongs to the parent ledger fast path"
    );
    assert!(response.is_none());
}

#[given("the child dispatch loop is occupied by a turn")]
fn child_dispatch_busy(world: &mut QuectoWorld) {
    // Like the parent case: the fast path never touches the dispatch loop, so
    // the assertion is that the sync is answered without queuing behind it.
    world.subagent_liveness_intercept = None;
}

#[when("its feed client sends a plain sync for the committed ledger")]
fn send_direct_feed_sync(world: &mut QuectoWorld) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let (served_inline, response) = rt.block_on(cli::busy_reader_dispatch(
        r#"{"type":"sync","id":"feed-1","epoch":1,"sinceRev":0}"#,
    ));
    world.subagent_liveness_intercept = Some((served_inline, response));
}

#[then("the sync should be answered inline without queuing behind the turn")]
fn sync_answered_inline(world: &mut QuectoWorld) {
    let (served_inline, _) = world
        .subagent_liveness_intercept
        .as_ref()
        .expect("dispatch result");
    assert!(
        served_inline,
        "the direct child-feed sync must never queue behind the dispatch loop"
    );
}

#[then("the sync response should carry the committed messages")]
fn sync_carries_committed(world: &mut QuectoWorld) {
    let (_, response) = world
        .subagent_liveness_intercept
        .as_ref()
        .expect("dispatch result");
    let response = response.as_ref().expect("a response line");
    assert_eq!(response["command"], "sync");
    assert_eq!(response["id"], "feed-1");
    assert_eq!(response["success"], true);
    let messages = response["data"]["messages"].as_array().expect("messages");
    assert!(
        messages.iter().any(|m| m["content"] == "committed"),
        "committed ledger content must be served while busy, got: {response}"
    );
}
