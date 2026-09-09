//! Step definitions for `tui_admission_waiting.feature` (#1679 P4 slice 3):
//! drives the real App through the headless harness with wire-shaped
//! `admission_state_changed` lines and `get_state` responses.

use crate::{TuiParityHarness, TuiWorld};
use cucumber::{given, then, when};
use quecto_tui::protocol::client::Event;
use quecto_tui::shell::app::tui_harness::{TuiHarness, subagent_with_socket, subagents_changed};

fn harness(world: &mut TuiWorld) -> &mut TuiHarness {
    &mut world.tui_parity.as_mut().expect("harness").0
}

#[given("a fresh TUI harness for admission scenarios")]
fn given_fresh(world: &mut TuiWorld) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let h = rt.block_on(TuiHarness::new());
    world.tui_parity_rt = Some(rt);
    world.tui_parity = Some(TuiParityHarness(h));
}

#[given(expr = "a running sub-agent {string} is on the roster")]
fn given_subagent(world: &mut TuiWorld, id: String) {
    let ev = subagents_changed(vec![subagent_with_socket(&id, "running", None, None)]);
    harness(world).event(ev);
}

#[when("the agent starts a run")]
fn when_run_starts(world: &mut TuiWorld) {
    harness(world).event(Event::AgentStart);
}

#[when("the agent ends the run")]
fn when_run_ends(world: &mut TuiWorld) {
    harness(world).event(Event::AgentEnd {
        messages: vec![],
        message_refs: vec![],
    });
}

#[when(
    expr = "the agent reports its admission view as waiting for {int} seconds in group {string}"
)]
fn when_waiting(world: &mut TuiWorld, seconds: u64, group: String) {
    let line = format!(
        r#"{{"type":"admission_state_changed","admission":{{"waiting":1,"admitted":0,"longestWaitSeconds":{seconds},"groups":[{{"group":"{group}"}}],"counters":{{"completed":0,"refused":0,"cancelled":0,"abandoned":0}},"hidden":0,"revision":1}}}}"#
    );
    harness(world).event_line(&line);
}

#[when("the agent reports its admission view as admitted")]
fn when_admitted(world: &mut TuiWorld) {
    harness(world).event_line(
        r#"{"type":"admission_state_changed","admission":{"waiting":0,"admitted":1,"groups":[{"group":"anthropic"}],"counters":{"completed":0,"refused":0,"cancelled":0,"abandoned":0},"hidden":0,"revision":2}}"#,
    );
}

#[when(expr = "the agent reports a {int} second cooldown for group {string}")]
fn when_cooldown(world: &mut TuiWorld, seconds: u64, group: String) {
    let line = format!(
        r#"{{"type":"admission_state_changed","admission":{{"waiting":0,"admitted":0,"groups":[{{"group":"{group}","cooldown":{{"state":"until","remainingSeconds":{seconds}}}}}],"hidden":0,"revision":3}}}}"#
    );
    harness(world).event_line(&line);
}

#[when(expr = "the parent forwards {string} waiting for admission for {int} seconds")]
fn when_child_waits(world: &mut TuiWorld, id: String, seconds: u64) {
    let line = format!(
        r#"{{"type":"admission_state_changed","agent_id":"{id}","parent_id":"root","admission":{{"waiting":1,"admitted":0,"longestWaitSeconds":{seconds},"groups":[{{"group":"anthropic"}}],"hidden":0,"revision":1}}}}"#
    );
    harness(world).event_line(&line);
}

#[when(expr = "the parent forwards {string} with nothing waiting")]
fn when_child_clear(world: &mut TuiWorld, id: String) {
    let line = format!(
        r#"{{"type":"admission_state_changed","agent_id":"{id}","parent_id":"root","admission":{{"waiting":0,"admitted":0,"groups":[],"hidden":0,"revision":2}}}}"#
    );
    harness(world).event_line(&line);
}

#[when(expr = "a get_state response arrives with a waiting admission view of {int} seconds")]
fn when_get_state(world: &mut TuiWorld, seconds: u64) {
    harness(world).event(Event::Response {
        id: None,
        command: "get_state".into(),
        success: true,
        data: Some(serde_json::json!({
            "state": "thinking", "model": "m", "sessionKey": "cli:default",
            "progress": {"state": "waiting", "reason": "queued"}, "generation": 2,
            "admission": {"waiting": 1, "longestWaitSeconds": seconds, "groups": [{"group": "anthropic"}], "revision": 1}
        })),
        error: None,
    });
}

#[then(expr = "the master footer shows {string}")]
fn then_footer_shows(world: &mut TuiWorld, text: String) {
    let footer = harness(world).master_footer_text();
    assert!(footer.contains(&text), "footer: {footer}");
    assert!(footer.contains("⏳"), "admission indicator: {footer}");
}

#[then("the master footer shows no admission label")]
fn then_footer_clear(world: &mut TuiWorld) {
    let footer = harness(world).master_footer_text();
    assert!(
        !footer.contains("admission") && !footer.contains("⏳"),
        "footer: {footer}"
    );
}

#[then(expr = "the working spinner says {string}")]
fn then_spinner(world: &mut TuiWorld, text: String) {
    let message = harness(world).spinner_message();
    assert_eq!(message.as_deref(), Some(text.as_str()));
}

#[then(expr = "the sub-agent panel row for {string} shows {string}")]
fn then_panel_shows(world: &mut TuiWorld, id: String, text: String) {
    let panel = harness(world).left_panel();
    let row = panel
        .lines()
        .find(|l| l.contains(&id))
        .unwrap_or_else(|| panic!("no row for {id}: {panel}"));
    assert!(row.contains(&text), "row: {row}");
}

#[then(expr = "the sub-agent panel row for {string} shows no admission label")]
fn then_panel_clear(world: &mut TuiWorld, id: String) {
    let panel = harness(world).left_panel();
    let row = panel
        .lines()
        .find(|l| l.contains(&id))
        .unwrap_or_else(|| panic!("no row for {id}: {panel}"));
    assert!(
        !row.contains("admission") && !row.contains("⏳"),
        "row: {row}"
    );
}
