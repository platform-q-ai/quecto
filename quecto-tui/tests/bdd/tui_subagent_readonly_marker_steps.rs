//! Step definitions for `tui_subagent_readonly_marker.feature` (#966).
//!
//! These drive the real TUI render path through the headless render harness and
//! assert on observable output in the left sub-agent panel.

use super::*;
use quecto_tui::interface::app::tui_harness::{
    TuiHarness, subagent, subagent_readonly, subagents_changed,
};
use quecto_tui::interface::theme::OBSERVER_GLYPH;
use quecto_tui::protocol::client::Event;

async fn build(agents: &[(String, bool)]) -> TuiHarness {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    push_agents(&mut h, agents);
    h
}

fn push_agents(h: &mut TuiHarness, agents: &[(String, bool)]) {
    let events: Vec<_> = agents
        .iter()
        .map(|(id, ro)| {
            if *ro {
                subagent_readonly(id, "running", Some(("active", 1, 3)), None)
            } else {
                subagent(id, "running", Some(("active", 1, 3)))
            }
        })
        .collect();
    h.event(subagents_changed(events));
}

fn init(world: &mut TuiWorld, agents: &[(String, bool)]) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let h = rt.block_on(build(agents));
    world.tui_expected_subagents = agents.to_vec();
    world.tui_parity_rt = Some(rt);
    world.tui_parity = Some(TuiParityHarness(h));
}

fn drive<R>(world: &mut TuiWorld, f: impl FnOnce(&mut TuiHarness) -> R) -> R {
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("harness runtime")
        .handle()
        .clone();
    let _guard = handle.enter();
    let h = &mut world.tui_parity.as_mut().expect("harness").0;
    f(h)
}

fn left_panel(world: &mut TuiWorld) -> String {
    drive(world, |h| h.left_panel())
}

fn row_for<'a>(panel: &'a str, id: &str) -> Option<&'a str> {
    panel.lines().find(|line| line.contains(id))
}

fn observer_rows(panel: &str) -> Vec<&str> {
    panel
        .lines()
        .filter(|line| line.contains(OBSERVER_GLYPH))
        .collect()
}

// ── Given ────────────────────────────────────────────────────────────────────

#[given(expr = "a sub-agent-first TUI tracking a read-only sub-agent {string}")]
fn given_readonly(world: &mut TuiWorld, id: String) {
    init(world, &[(id, true)]);
}

#[given(expr = "a sub-agent-first TUI tracking a read-write sub-agent {string}")]
fn given_readwrite(world: &mut TuiWorld, id: String) {
    init(world, &[(id, false)]);
}

#[given(
    expr = "a sub-agent-first TUI tracking a read-only sub-agent {string} and a read-write sub-agent {string}"
)]
fn given_mixed(world: &mut TuiWorld, ro: String, rw: String) {
    init(world, &[(ro, true), (rw, false)]);
}

// ── When ─────────────────────────────────────────────────────────────────────

#[when("the operator views the left sub-agent panel")]
fn when_views_panel(world: &mut TuiWorld) {
    let panel = left_panel(world);
    assert!(!panel.trim().is_empty(), "left panel should render");
}

#[when(expr = "sub-agent {string} leaves")]
fn when_leaves(world: &mut TuiWorld, id: String) {
    world
        .tui_expected_subagents
        .retain(|(agent_id, _)| agent_id != &id);
    let agents = world.tui_expected_subagents.clone();
    drive(world, |h| push_agents(h, &agents));
}

// ── Then ─────────────────────────────────────────────────────────────────────

#[then(expr = "the left panel shows sub-agent {string} as an observer")]
fn then_agent_is_observer(world: &mut TuiWorld, id: String) {
    let panel = left_panel(world);
    let row = row_for(&panel, &id)
        .unwrap_or_else(|| panic!("sub-agent {id} should appear in the left panel:\n{panel}"));
    assert!(
        row.contains(OBSERVER_GLYPH),
        "sub-agent {id} must be shown as an observer, got panel:\n{panel}"
    );
}

#[then(expr = "the left panel shows sub-agent {string} without an observer marker")]
fn then_agent_is_not_observer(world: &mut TuiWorld, id: String) {
    let panel = left_panel(world);
    let row = row_for(&panel, &id)
        .unwrap_or_else(|| panic!("sub-agent {id} should appear in the left panel:\n{panel}"));
    assert!(
        !row.contains(OBSERVER_GLYPH),
        "sub-agent {id} must not be shown as an observer, got panel:\n{panel}"
    );
}

#[then(expr = "only sub-agent {string} is shown as an observer")]
fn then_only_agent_is_observer(world: &mut TuiWorld, id: String) {
    let panel = left_panel(world);
    let rows = observer_rows(&panel);
    assert_eq!(
        rows.len(),
        1,
        "exactly one sub-agent should be shown as an observer, got {} rows:\n{panel}",
        rows.len()
    );
    assert!(
        rows[0].contains(id.as_str()),
        "the observer marker ({OBSERVER_GLYPH}) must belong to sub-agent {id}, got panel:\n{panel}"
    );
    for (agent_id, read_only) in &world.tui_expected_subagents {
        let row = row_for(&panel, agent_id).unwrap_or_else(|| {
            panic!("tracked sub-agent {agent_id} should appear in the left panel:\n{panel}")
        });
        assert_eq!(
            row.contains(OBSERVER_GLYPH),
            *read_only,
            "observer status for tracked sub-agent {agent_id} should match read-only status:\n{panel}"
        );
    }
}

#[then(expr = "the left panel no longer shows sub-agent {string}")]
fn then_agent_absent(world: &mut TuiWorld, id: String) {
    let panel = left_panel(world);
    assert!(
        row_for(&panel, &id).is_none(),
        "sub-agent {id} should no longer appear in the left panel:\n{panel}"
    );
}

#[then("the left panel shows no observer sub-agents")]
fn then_no_observers(world: &mut TuiWorld) {
    let panel = left_panel(world);
    let rows = observer_rows(&panel);
    assert!(
        rows.is_empty(),
        "no sub-agent should be shown as an observer, got {} rows:\n{panel}",
        rows.len()
    );
}
