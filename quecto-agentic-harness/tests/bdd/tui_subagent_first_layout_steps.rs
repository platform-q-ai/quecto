//! Step definitions for `tui_subagent_first_layout.feature` (#820).
//!
//! These drive the REAL TUI render path through the headless render harness
//! (`quecto_tui::interface::app::tui_harness`), exposed via the `test-harness`
//! feature. Each step asserts on observable rendered output (the main-pane vs
//! the bottom stack), not internal mechanics.

use super::*;
use quecto_tui::infrastructure::client::Event;
use quecto_tui::interface::app::tui_harness::{TuiHarness, subagent, subagents_changed};
use quecto_tui::interface::utils::visible_width;

/// Build a sub-agent-first harness optionally tracking sub-agent `a1` whose own
/// workflow (issue #820) has been routed into its session.
async fn build(with_subagent: bool, subagent_id: &str) -> TuiHarness {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    if with_subagent {
        h.event(subagents_changed(vec![subagent(
            subagent_id,
            "running",
            Some(("active", 3, 5)),
        )]));
        h.route(subagent_id, workflow_event(subagent_id));
    }
    h
}

/// A workflow_state event for `agent` with a `green` current phase at 3/5 and an
/// active issue (#820), so the compact bar has content to render.
fn workflow_event(agent: &str) -> Event {
    Event::WorkflowState {
        agent_id: Some(agent.to_string()),
        steps: vec![
            serde_json::json!({"index":1,"label":"a","phase":"red","done":true}),
            serde_json::json!({"index":2,"label":"b","phase":"red","done":true}),
            serde_json::json!({"index":3,"label":"c","phase":"green","done":true}),
            serde_json::json!({"index":4,"label":"d","phase":"green","done":false}),
            serde_json::json!({"index":5,"label":"e","phase":"review","done":false}),
        ],
        progress: serde_json::json!({"done":3,"total":5,"percent":60}),
        active_issue: Some(serde_json::json!({"number":820,"title":"layoutwork"})),
        mode: Some("active".to_string()),
        active_template: None,
        available_templates: None,
    }
}

fn init(world: &mut QuectoWorld, with_subagent: bool, subagent_id: &str) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let h = rt.block_on(build(with_subagent, subagent_id));
    world.tui_parity_rt = Some(rt);
    world.tui_parity = Some(TuiParityHarness(h));
}

fn drive<R>(world: &mut QuectoWorld, f: impl FnOnce(&mut TuiHarness) -> R) -> R {
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

/// The bottom stack, ANSI-stripped.
fn bottom(world: &mut QuectoWorld) -> String {
    drive(world, |h| h.bottom_stack())
}

/// The main-pane (top) region of the frame, ANSI-stripped.
fn top(world: &mut QuectoWorld) -> String {
    drive(world, |h| h.main_pane())
}

// ── Given ────────────────────────────────────────────────────────────────────

#[given("a sub-agent-first TUI with no sub-agents")]
fn given_no_subagents(world: &mut QuectoWorld) {
    init(world, false, "");
}

#[given(expr = "a sub-agent-first TUI tracking sub-agent {string} with its own workflow")]
fn given_tracking(world: &mut QuectoWorld, id: String) {
    init(world, true, &id);
}

// The `When I select sub-agent "..."` step is shared with the parity steps
// module (`tui_subagent_parity_steps`), which drives the same world fields.

// ── Then ──────────────────────────────────────────────────────────────────────

#[then("the left panel shows the master row")]
fn then_master_row(world: &mut QuectoWorld) {
    let frame = drive(world, |h| h.full_frame());
    assert!(
        frame.contains("Master"),
        "the always-on panel must show the master row, got:\n{frame}"
    );
}

#[then("the main pane shows a boxed workflow bar")]
fn then_main_pane_boxed(world: &mut QuectoWorld) {
    let top = top(world);
    assert!(
        (top.contains('┌') || top.contains('╭')) && top.contains("Step 4/5"),
        "the main pane must show a boxed one-line workflow bar (current step n/total), got:\n{top}"
    );
}

#[then("the main pane shows a boxed workflow bar aligned to the tool/message content column")]
fn then_main_pane_boxed_aligned(world: &mut QuectoWorld) {
    drive(world, |h| {
        let frame = h.full_frame();
        let border = frame
            .lines()
            .find(|line| line.contains('┌'))
            .expect("workflow box top border should render");
        let header = frame
            .lines()
            .find(|line| line.contains("quecto-tui"))
            .expect("header should render with the main-pane divider");
        let divider = header.find('│').expect("header should include divider");
        let panel_w = visible_width(&header[..divider]);
        // The workflow box's left border column must line up with the tool/message
        // content column (one space after the divider), and its width must equal
        // the body width — not consume the gutter.
        // Use harness terminal width (independent source) instead of deriving from frame
        let terminal_w = h.terminal_width();
        let expected = terminal_w - panel_w - 1 - 1; // panel + divider + gutter
        let border_segment = &border[border.find('┌').expect("border starts with ┌")..];
        assert_eq!(
            visible_width(border_segment),
            expected,
            "workflow box must equal the body/tool width (not consume the gutter), got:\n{frame}"
        );
        assert!(
            border.contains("│ ┌"),
            "workflow box must start one column after the divider (aligned to tool/message content column), got:\n{frame}"
        );
    });
}

#[then("the bottom stack no longer shows the workflow bar")]
fn then_bottom_no_workflow(world: &mut QuectoWorld) {
    let bottom = bottom(world);
    assert!(
        !bottom.contains("Workflow")
            && !bottom.contains("Ctrl+Shift+A")
            && !bottom.contains("#820"),
        "the workflow bar must be gone from the bottom stack, got:\n{bottom}"
    );
}

#[then("the bottom stack no longer shows the sub-agent bar")]
fn then_bottom_no_subagent_bar(world: &mut QuectoWorld) {
    let bottom = bottom(world);
    assert!(
        !bottom.contains("Subagents"),
        "the sub-agent bar must be gone from the bottom stack, got:\n{bottom}"
    );
}
