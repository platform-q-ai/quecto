//! RED-phase behavioural tests for the **sub-agent-first default layout**
//! (#820), driven through the headless render harness.
//!
//! These pin the acceptance criteria from the issue:
//!   * the left panel is ALWAYS visible once connected — the Master row shows
//!     even with no sub-agents (not gated on `!subagent_local.is_empty()`);
//!   * panel rows carry NO status dot/glyph; the NAME TEXT is coloured by status
//!     (running = green, idle = orange/yellow, errored = red);
//!   * selecting an agent fills the MAIN PANE: a title line plus a BOXED,
//!     single-line yellow workflow bar (progress + phase + n/total) — no
//!     phase-pills line and no hints line;
//!   * the old sub-agent bar and the workflow bar no longer render in the bottom
//!     stack (`compose_bottom`);
//!   * `compose_frame` stays render-idempotent (no flash) with the panel.
//!
//! They drive not-yet-existing layout behaviour on purpose (TDD RED).

use super::tui_harness::*;
use crate::infrastructure::client::{Event, SubagentInfoEvent, SubagentWorkflow};
use crate::interface::ansi::strip_ansi;

/// Panel-row status glyphs that the sub-agent-first design removes entirely.
const GLYPHS: &[&str] = &["●", "✓", "✗", "○", "•"];

/// A `SubagentInfoEvent` with an explicit status and parent (no socket).
fn child(id: &str, status: &str, parent: Option<&str>) -> SubagentInfoEvent {
    SubagentInfoEvent {
        agent_id: id.to_string(),
        status: status.to_string(),
        last_tool: None,
        last_error: None,
        pid: 0,
        socket_path: None,
        parent_id: parent.map(|p| p.to_string()),
        workflow: Some(SubagentWorkflow {
            mode: "active".to_string(),
            steps_completed: 3,
            steps_total: 5,
        }),
    }
}

/// A workflow_state event for `agent` (None = master) with a `green` current
/// phase at 3/5 done and an active issue, so the compact bar has content.
fn workflow_event(agent: Option<&str>) -> Event {
    Event::WorkflowState {
        agent_id: agent.map(|s| s.to_string()),
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

/// The top (main-pane) region of the frame: everything above the bottom stack.
/// Delegates to the harness so the body-width split matches the live frame
/// (#820 review — both must slice at the reduced body width, not full width).
fn top_region(h: &mut TuiHarness) -> String {
    h.main_pane()
}

// ── Always-on panel (Master row present even with no sub-agents) ─────────────

#[tokio::test]
async fn panel_visible_with_master_only() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    assert!(
        h.app_mut().subagent_panel_visible(),
        "the panel must be ALWAYS visible once connected (sub-agent-first), even with no sub-agents"
    );
}

#[tokio::test]
async fn master_row_shows_with_no_subagents() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    let frame = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(
        frame.contains("Master"),
        "the Master row must be present as the top panel row even with no sub-agents:\n{frame}"
    );
}

// ── Rows: no glyphs, status-coloured name ────────────────────────────────────

#[tokio::test]
async fn panel_rows_have_no_status_glyphs() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent(
        "worker",
        "running",
        Some(("active", 3, 5)),
    )]));
    let frame = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    let row = frame
        .lines()
        .find(|l| l.contains("worker"))
        .unwrap_or_else(|| panic!("worker row not found in:\n{frame}"))
        .to_string();
    for g in GLYPHS {
        assert!(
            !row.contains(g),
            "panel rows must carry NO status glyph ({g}); row was: {row:?}"
        );
    }
}

#[tokio::test]
async fn running_name_is_green() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent(
        "runr",
        "running",
        Some(("active", 3, 5)),
    )]));
    let raw = h.app_mut().compose_frame().join("\n");
    let row = raw
        .lines()
        .find(|l| l.contains("runr"))
        .unwrap_or_else(|| panic!("runr row not found"))
        .to_string();
    assert!(
        row.contains("\x1b[32m"),
        "a running agent's NAME must be coloured green; raw row: {row:?}"
    );
}

#[tokio::test]
async fn idle_name_is_orange() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![child("idlr", "idle", None)]));
    let raw = h.app_mut().compose_frame().join("\n");
    let row = raw
        .lines()
        .find(|l| l.contains("idlr"))
        .unwrap_or_else(|| panic!("idlr row not found"))
        .to_string();
    assert!(
        row.contains("\x1b[33m"),
        "an idle agent's NAME must be coloured orange/yellow; raw row: {row:?}"
    );
    assert!(
        !row.contains("\x1b[32m"),
        "an idle agent's NAME must NOT be green; raw row: {row:?}"
    );
}

#[tokio::test]
async fn errored_name_is_red() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![child("errr", "error", None)]));
    let raw = h.app_mut().compose_frame().join("\n");
    let row = raw
        .lines()
        .find(|l| l.contains("errr"))
        .unwrap_or_else(|| panic!("errr row not found"))
        .to_string();
    assert!(
        row.contains("\x1b[31m"),
        "an errored agent's NAME must be coloured red; raw row: {row:?}"
    );
    assert!(
        !row.contains("\x1b[32m"),
        "an errored agent's NAME must NOT be green; raw row: {row:?}"
    );
}

// ── Per-row elapsed timers (running live m:ss, idle `idle m:ss` prefix) ───────

#[tokio::test]
async fn running_row_shows_mss_elapsed() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent(
        "runr",
        "running",
        Some(("active", 3, 5)),
    )]));
    let frame = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    let row = frame
        .lines()
        .find(|l| l.contains("runr"))
        .unwrap_or_else(|| panic!("runr row not found in:\n{frame}"))
        .to_string();
    // A live `m:ss` timer: a digit, a colon, then two digits.
    let has_mss = row
        .char_indices()
        .any(|(i, c)| c == ':' && surrounding_is_mss(&row, i));
    assert!(
        has_mss,
        "a running row must show a live m:ss elapsed timer; row was: {row:?}"
    );
}

/// True when position `i` (a `:`) is flanked by `d:dd` digits — an `m:ss` clock.
fn surrounding_is_mss(row: &str, colon: usize) -> bool {
    let bytes = row.as_bytes();
    colon >= 1
        && colon + 2 < bytes.len()
        && bytes[colon - 1].is_ascii_digit()
        && bytes[colon + 1].is_ascii_digit()
        && bytes[colon + 2].is_ascii_digit()
}

#[tokio::test]
async fn idle_row_shows_idle_prefix() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![child("idlr", "idle", None)]));
    let frame = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    let row = frame
        .lines()
        .find(|l| l.contains("idlr"))
        .unwrap_or_else(|| panic!("idlr row not found in:\n{frame}"))
        .to_string();
    assert!(
        row.contains("idle "),
        "an idle row's elapsed must carry the `idle ` prefix; row was: {row:?}"
    );
}

// ── Main pane: boxed, single-line workflow bar for the selected agent ────────

#[tokio::test]
async fn main_pane_shows_boxed_workflow_for_selected_agent() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent(
        "worker",
        "running",
        Some(("active", 3, 5)),
    )]));
    h.app_mut().select_agent(Some("worker"));
    h.app_mut()
        .route_subagent_event("worker", workflow_event(Some("worker")));
    let top = top_region(&mut h);
    assert!(
        top.contains('┌') || top.contains('╭'),
        "the relocated workflow bar must be BOXED in the main pane:\n{top}"
    );
    assert!(
        top.contains("3/5"),
        "the boxed workflow bar must show progress n/total in the main pane:\n{top}"
    );
    assert!(
        top.contains("#820"),
        "the main-pane title line must show the selected agent's #issue:\n{top}"
    );
}

#[tokio::test]
async fn main_pane_title_renders_when_agent_has_no_workflow() {
    // #820 review (Finding 2): the title line must ALWAYS render for the selected
    // agent; only the boxed workflow bar is conditional on a workflow.
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent("worker", "running", None)]));
    h.app_mut().select_agent(Some("worker"));
    // No workflow_state routed → compact line is None.
    let top = strip_ansi(&top_region(&mut h));
    assert!(
        top.contains("worker"),
        "the main-pane TITLE must render for the selected agent even with no workflow:\n{top}"
    );
    assert!(
        !top.contains('┌') && !top.contains('╭'),
        "no boxed workflow bar should render when the agent has no workflow:\n{top}"
    );
}

#[tokio::test]
async fn boxed_workflow_bar_drops_pills_and_hints() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent(
        "worker",
        "running",
        Some(("active", 3, 5)),
    )]));
    h.app_mut().select_agent(Some("worker"));
    h.app_mut()
        .route_subagent_event("worker", workflow_event(Some("worker")));
    let top = top_region(&mut h);
    assert!(
        !top.contains("Ctrl+Shift+A") && !top.contains("nudge:"),
        "the relocated workflow bar must DROP the hints line:\n{top}"
    );
    // The pill markers `○`/`●` appear ONLY on the dropped phase-pills line (the
    // panel rows carry no glyphs and the active caret is `▸`), so their absence
    // proves the pills line is gone, not just the hints line.
    assert!(
        !top.contains('○') && !top.contains('●'),
        "the boxed bar must DROP the phase-pills line:\n{top}"
    );
}

// ── Bottom stack: old sub-agent bar + workflow bar removed ───────────────────

#[tokio::test]
async fn bottom_stack_has_no_subagent_or_workflow_bars() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    // Master workflow active (would have rendered the bottom workflow bar).
    h.event(workflow_event(None));
    h.event(subagents_changed(vec![subagent(
        "worker",
        "running",
        Some(("active", 3, 5)),
    )]));
    let width = h.app_mut().terminal.width;
    let bottom = strip_ansi(&h.app_mut().compose_bottom(width).join("\n"));
    assert!(
        !bottom.contains("Subagents"),
        "the old sub-agent bar must be removed from the bottom stack:\n{bottom}"
    );
    assert!(
        !bottom.contains("Workflow") && !bottom.contains("Ctrl+Shift+A"),
        "the workflow bar must be removed from the bottom stack:\n{bottom}"
    );
    assert!(
        !bottom.contains("#820"),
        "the workflow bar (issue line) must no longer render in the bottom stack:\n{bottom}"
    );
}

#[tokio::test]
async fn editor_and_footer_remain_in_bottom_stack() {
    // The removals must NOT take the editor/footer with them.
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent(
        "worker",
        "running",
        Some(("active", 3, 5)),
    )]));
    let width = h.app_mut().terminal.width;
    let bottom = h.app_mut().compose_bottom(width);
    assert!(
        !bottom.is_empty(),
        "the bottom stack must still render the editor/footer after the bar removals"
    );
}

// ── No flash ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn master_only_compose_frame_is_idempotent() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    let a = h.app_mut().compose_frame();
    let b = h.app_mut().compose_frame();
    assert_eq!(
        a, b,
        "compose_frame must be render-idempotent (no flash) with the always-on panel"
    );
}

#[tokio::test]
async fn selected_agent_compose_frame_is_idempotent() {
    // With a sub-agent selected, the panel rows + main-pane title both render
    // elapsed timers. compose_frame must sample the clock ONCE per frame and
    // thread it through, so two back-to-back composes are byte-identical even
    // across a possible second boundary (#820 review — render-idempotency).
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent(
        "worker",
        "running",
        Some(("active", 3, 5)),
    )]));
    h.app_mut().select_agent(Some("worker"));
    let a = h.app_mut().compose_frame();
    let b = h.app_mut().compose_frame();
    assert_eq!(
        a, b,
        "elapsed timers must not re-sample the clock mid-frame (no flash)"
    );
}

#[tokio::test]
async fn main_pane_title_sanitizes_status_text() {
    // The status string crosses a trust boundary (kernel/daemon over UDS) and is
    // printed as title text. It must be stripped of terminal control sequences,
    // mirroring the adjacent agent-name path (#820 security review).
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![child(
        "worker",
        "run\u{1b}[2Jning",
        None,
    )]));
    h.app_mut().select_agent(Some("worker"));
    // Inspect the RAW (un-stripped) frame: theme colours add ANSI, but only an
    // unsanitized status would inject the `[2J` clear-screen CSI payload.
    let raw = h.full_frame_raw();
    assert!(
        !raw.contains("2J"),
        "the clear-screen CSI from the status field must be stripped before render"
    );
}
