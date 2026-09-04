//! RED-phase behavioural tests for the **sub-agent-first default layout**
//! (#820), driven through the headless render harness.
//!
//! These pin the acceptance criteria from the issue:
//!   * the left panel is ALWAYS visible once connected — the Master row shows
//!     even with no sub-agents (not gated on `!subagent_local.is_empty()`);
//!   * panel rows carry NO status dot/glyph; the NAME TEXT is coloured by status
//!     (running = green, idle = orange/yellow, errored = red);
//!   * selecting an agent fills the MAIN PANE: a title line plus a full-width
//!     workflow status bar (progress + phase + n/total) — no phase-pills line
//!     and no hints line;
//!   * the old sub-agent bar and the workflow bar no longer render in the bottom
//!     stack (`compose_bottom`);
//!   * `compose_frame` stays render-idempotent (no flash) with the panel.
//!
//! They drive not-yet-existing layout behaviour on purpose (TDD RED).

use super::app_subagent_panel::controller_subagent_panel_helpers::panel_markers;
use super::tui_harness::*;
use crate::components::ansi::strip_ansi;
use crate::protocol::client::{Event, SubagentInfoEvent, SubagentWorkflow};

/// Panel-row status glyphs that the sub-agent-first design removes entirely.
const GLYPHS: &[&str] = &["●", "✓", "✗", "○", "•"];

/// A `SubagentInfoEvent` with an explicit status and parent (no socket).
fn child(id: &str, status: &str, parent: Option<&str>) -> SubagentInfoEvent {
    SubagentInfoEvent {
        agent_uuid: None,
        display_name: None,
        agent_id: id.to_string(),
        status: status.to_string(),
        last_tool: None,
        last_error: None,
        compact: false,
        pid: 0,
        socket_path: None,
        parent_id: parent.map(|p| p.to_string()),
        workflow: Some(SubagentWorkflow {
            mode: "active".to_string(),
            steps_completed: 3,
            steps_total: 5,
        }),
        read_only: false,
        execution_backend: None,
        environment: None,
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

/// The retained session's workflow bar for `id` (panics when absent).
fn session_bar<'a>(
    app: &'a mut super::App,
    id: &str,
) -> &'a crate::components::workflow_bar::WorkflowBarState {
    &app.ac().roster.sessions.get(id).unwrap().workflow_bar
}

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
    // Inspect the PANEL render only (not the full frame), so colour codes from
    // the main pane on the same terminal row can't contaminate the assertion.
    let raw = h
        .app_mut()
        .render_subagent_panel(30, 24, tokio::time::Instant::now())
        .join("\n");
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
    // Inspect the PANEL render only (not the full frame), so colour codes from
    // the main pane on the same terminal row can't contaminate the assertion.
    let raw = h
        .app_mut()
        .render_subagent_panel(30, 24, tokio::time::Instant::now())
        .join("\n");
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
    // Inspect the PANEL render only (not the full frame), so colour codes from
    // the main pane on the same terminal row can't contaminate the assertion.
    let raw = h
        .app_mut()
        .render_subagent_panel(30, 24, tokio::time::Instant::now())
        .join("\n");
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

/// Strip ANSI from each panel-render line (panel only — no main-pane content).
fn panel_lines(h: &mut TuiHarness) -> Vec<String> {
    h.app_mut()
        .render_subagent_panel(30, 24, tokio::time::Instant::now())
        .iter()
        .map(|l| strip_ansi(l))
        .collect()
}

#[tokio::test]
async fn idle_row_shows_bare_timer_no_status_word() {
    // Status is carried by the name COLOUR now (yellow = idle), so the panel row
    // shows only a bare `m:ss` timer — no `idle`/`ran` word.
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![child("idlr", "idle", None)]));
    let lines = panel_lines(&mut h);
    let row = lines
        .iter()
        .find(|l| l.contains("idlr"))
        .unwrap_or_else(|| panic!("idlr row not found in:\n{}", lines.join("\n")))
        .to_string();
    assert!(
        !row.contains("idle") && !row.contains("ran"),
        "panel rows must not carry a status word; row was: {row:?}"
    );
    assert!(
        row.bytes()
            .enumerate()
            .any(|(i, b)| b == b':' && surrounding_is_mss(&row, i)),
        "an idle row must still show a bare m:ss timer; row was: {row:?}"
    );
}

#[tokio::test]
async fn workflowed_agent_renders_step_bar_beneath_name() {
    // `child()` carries a 3/5 workflow, so the agent renders a ASCII per-step bar
    // on the row DIRECTLY BENEATH its name row.
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![child("worker", "running", None)]));
    let lines = panel_lines(&mut h);
    let name_idx = lines
        .iter()
        .position(|l| l.contains("worker"))
        .unwrap_or_else(|| panic!("worker row not found in:\n{}", lines.join("\n")));
    let bar = &lines[name_idx + 1];
    assert!(
        bar.contains("===>."),
        "the row beneath the name should be a ASCII step bar; got: {bar:?}"
    );
}

#[tokio::test]
async fn selection_uses_left_accent_bar_one_line_tall() {
    // Selection is a single ▌ bar in column 0 of the NAME row only — not a
    // full-row reverse, and NOT on the workflow-bar row beneath it. The bar
    // only renders while the panel has keyboard focus.
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![child("worker", "running", None)]));
    h.app_mut().select_agent(Some("worker"));
    h.app_mut().subagents.focus = super::Focus::Panel;
    let lines = panel_lines(&mut h);
    let name_idx = lines
        .iter()
        .position(|l| l.starts_with('▌') && l.contains("worker"))
        .unwrap_or_else(|| panic!("selected ▌ worker row not found in:\n{}", lines.join("\n")));
    assert!(
        !lines[name_idx + 1].starts_with('▌'),
        "the bar row beneath a selected agent must NOT carry the ▌ (one line tall): {:?}",
        lines[name_idx + 1]
    );
}

// ── Main pane: title + compact progress with separator rules (#1309) ──────────

fn is_rule_row(line: &str) -> bool {
    let segment = line.rsplit_once("│ ").map(|(_, s)| s).unwrap_or(line);
    let t = segment.trim();
    !t.is_empty() && t.chars().all(|c| c == '─')
}

fn compact_progress_has_separators(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().collect();
    let Some(idx) = lines
        .iter()
        .position(|l| l.contains("Step 4/5") || l.contains("3/5"))
    else {
        return false;
    };
    idx > 0
        && is_rule_row(lines[idx - 1])
        && idx + 1 < lines.len()
        && is_rule_row(lines[idx + 1])
        && lines.iter().filter(|l| is_rule_row(l)).count() == 2
}

#[tokio::test]
async fn main_pane_shows_compact_progress_with_separators_for_selected_agent() {
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
    let stripped = strip_ansi(&top);
    // #1288: compact progress; #1309: top/bottom separator rules; #1246: no pills/hints.
    assert!(
        compact_progress_has_separators(&stripped),
        "compact progress must be framed by separator rules:\n{top}"
    );
    assert!(
        stripped.contains("Step 4/5") || stripped.contains("3/5"),
        "live progress required:\n{top}"
    );
    assert!(top.contains("#820"), "issue in title:\n{top}");
    assert!(
        stripped.contains("worker · running"),
        "selected-agent title must remain visible:\n{top}"
    );
    assert!(
        !top.contains("Ctrl+Shift+A")
            && !top.contains("nudge:")
            && !top.contains('○')
            && !top.contains('●'),
        "no pills/hints:\n{top}"
    );
}

#[tokio::test]
async fn main_pane_title_renders_when_agent_has_no_workflow() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent("worker", "running", None)]));
    h.app_mut().select_agent(Some("worker"));
    let top = strip_ansi(&top_region(&mut h));
    assert!(top.contains("worker"), "title without workflow:\n{top}");
    assert!(
        !top.contains('┌') && !top.contains('╭'),
        "no box chrome:\n{top}"
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

#[tokio::test]
async fn forwarded_grandchild_workflow_routes_by_event_agent_id_not_connection() {
    // #840: while viewing child C, C's connection carries BOTH C's own
    // workflow_state AND the kernel-forwarded grandchild G workflow (re-stamped
    // with G's own inner agent_id). The grandchild's event must update G's
    // session bar and never overwrite C's bar.
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    // Full-tree broadcast (#815) tracks both the child and the grandchild.
    h.event(subagents_changed(vec![
        subagent("C", "running", Some(("active", 1, 3))),
        subagent("G", "running", Some(("active", 1, 3))),
    ]));
    h.app_mut().select_agent(Some("C"));
    // C's OWN workflow: the connected agent's own event carries `agent_id: None`
    // (per the type contract in `client.rs`), so routing must fall back to the
    // connection id. Issue 820.
    h.app_mut().route_subagent_event(
        "C",
        Event::WorkflowState {
            agent_id: None,
            steps: vec![],
            progress: serde_json::json!({"done": 3, "total": 5}),
            active_issue: Some(serde_json::json!({"number": 820, "title": "child"})),
            mode: Some("active".into()),
            active_template: None,
            available_templates: None,
        },
    );
    // The forwarded grandchild workflow arrives tagged for C's connection but
    // carries G's own inner agent_id: the shared `forwarded_workflow` helper
    // hardcodes issue 7.
    h.app_mut()
        .route_subagent_event("C", forwarded_workflow("G", 2, 4));
    let app = h.app_mut();
    assert_eq!(
        session_bar(app, "C").issue_number,
        Some(820),
        "child C's bar must keep C's own workflow, not the grandchild's"
    );
    assert_eq!(
        app.ac()
            .roster
            .sessions
            .get("G")
            .expect("grandchild G must get its own session")
            .workflow_bar
            .issue_number,
        Some(7),
        "grandchild G's forwarded workflow must land on G's own session bar"
    );
}

#[tokio::test]
async fn connected_agent_own_workflow_with_matching_inner_id_routes_to_self() {
    // #840 (Finding 1): the connected agent's own workflow_state may also arrive
    // with its inner agent_id equal to the connection id. That `Some(C)==conn`
    // shape must still land on C's own bar (it equals the fallback target).
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent(
        "C",
        "running",
        Some(("active", 1, 3)),
    )]));
    h.app_mut().select_agent(Some("C"));
    h.app_mut().route_subagent_event(
        "C",
        Event::WorkflowState {
            agent_id: Some("C".into()),
            steps: vec![],
            progress: serde_json::json!({"done": 3, "total": 5}),
            active_issue: Some(serde_json::json!({"number": 820, "title": "child"})),
            mode: Some("active".into()),
            active_template: None,
            available_templates: None,
        },
    );
    assert_eq!(
        session_bar(h.app_mut(), "C").issue_number,
        Some(820),
        "C's own workflow_state (inner id == connection id) must land on C's bar"
    );
}

#[tokio::test]
async fn forwarded_workflow_for_untracked_agent_is_dropped() {
    // #840 (Finding 2): a forwarded workflow_state whose inner agent_id is NOT a
    // tracked/retained agent must be dropped — it must neither create a phantom
    // session for the unknown id nor touch the connected child's bar.
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    // Only C is tracked; X is unknown.
    h.event(subagents_changed(vec![subagent(
        "C",
        "running",
        Some(("active", 1, 3)),
    )]));
    h.app_mut().select_agent(Some("C"));
    // Establish C's own bar first.
    h.app_mut().route_subagent_event(
        "C",
        Event::WorkflowState {
            agent_id: None,
            steps: vec![],
            progress: serde_json::json!({"done": 3, "total": 5}),
            active_issue: Some(serde_json::json!({"number": 820, "title": "child"})),
            mode: Some("active".into()),
            active_template: None,
            available_templates: None,
        },
    );
    // A forwarded workflow for an untracked grandchild X arrives on C's stream.
    h.app_mut()
        .route_subagent_event("C", forwarded_workflow("X", 2, 4));
    let app = h.app_mut();
    assert!(
        !app.ac().roster.sessions.contains_key("X"),
        "an untracked forwarded id must not create a phantom session"
    );
    assert_eq!(
        session_bar(app, "C").issue_number,
        Some(820),
        "an untracked forwarded workflow must not touch the connected child's bar"
    );
}

/// A `get_state` Response carrying a child's mid-workflow snapshot (#842 / #869),
/// shaped like the kernel's connect-time get_state (`progress` + `activeIssue`).
fn get_state_with_workflow(done: u32, total: u32, issue: u32) -> Event {
    Event::Response {
        id: Some("subagent-state".into()),
        command: "get_state".into(),
        success: true,
        data: Some(serde_json::json!({
            "workflow": {
                "progress": { "done": done, "total": total },
                "activeIssue": { "number": issue, "title": "child" },
                "mode": "active",
            }
        })),
        error: None,
    }
}

#[tokio::test]
async fn subagent_get_state_response_populates_workflow_bar() {
    // #869 (a): viewing a child mid-workflow, the connect-time get_state snapshot
    // carries the child's workflow — `route_subagent_event` must populate the
    // session bar from it, not wait for the next live workflow_state transition.
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent(
        "C",
        "running",
        Some(("active", 1, 3)),
    )]));
    h.app_mut().select_agent(Some("C"));
    h.app_mut()
        .route_subagent_event("C", get_state_with_workflow(3, 20, 869));
    let bar = session_bar(h.app_mut(), "C");
    assert_eq!(bar.done, 3, "get_state workflow `done` must reach the bar");
    assert_eq!(
        bar.total, 20,
        "get_state workflow `total` must reach the bar"
    );
    assert_eq!(
        bar.issue_number,
        Some(869),
        "get_state workflow issue must reach the bar"
    );
}

#[tokio::test]
async fn subagent_get_state_routes_by_inner_agent_id_not_connection() {
    // #869 (a) / #840: a get_state must land on the connected child only — it is
    // not a forwarded descendant event, so it uses the connection id and must not
    // create a phantom session nor be mis-routed.
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent(
        "C",
        "running",
        Some(("active", 1, 3)),
    )]));
    h.app_mut().select_agent(Some("C"));
    h.app_mut()
        .route_subagent_event("C", get_state_with_workflow(2, 9, 100));
    assert_eq!(
        session_bar(h.app_mut(), "C").total,
        9,
        "the connected child's get_state must populate ITS bar"
    );
}

#[tokio::test]
async fn live_workflow_state_renders_full_empty_markers_in_left_panel() {
    // #869 (b): a 3/20 workflow delivered as a live workflow_state for the viewed
    // child must update its LEFT-PANEL row to show 3 filled + 17 empty markers —
    // the panel must not collapse to only the filled markers, and must reflect the
    // child's own live progress (not just the last subagent_state_changed push).
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    // Child tracked WITHOUT a workflow snapshot yet (no panel bar initially).
    h.event(subagents_changed(vec![subagent("C", "running", None)]));
    h.app_mut().select_agent(Some("C"));
    h.app_mut().route_subagent_event(
        "C",
        Event::WorkflowState {
            agent_id: None,
            steps: vec![],
            progress: serde_json::json!({ "done": 3, "total": 20 }),
            active_issue: Some(serde_json::json!({ "number": 1, "title": "w" })),
            mode: Some("active".into()),
            active_template: None,
            available_templates: None,
        },
    );
    let frame = panel_lines(&mut h).join("\n");
    let (filled, empty) = panel_markers(&frame);
    assert_eq!(
        filled, 3,
        "3 completed steps render as 3 filled markers:\n{frame}"
    );
    assert_eq!(
        empty, 17,
        "17 incomplete steps must render as empty markers, not collapse:\n{frame}"
    );
}

#[tokio::test]
async fn get_state_snapshot_renders_full_empty_markers_in_left_panel() {
    // #869 (b): the same full filled+empty marker rendering must work from the
    // connect-time get_state snapshot, not only from live workflow_state.
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent("C", "running", None)]));
    h.app_mut().select_agent(Some("C"));
    h.app_mut()
        .route_subagent_event("C", get_state_with_workflow(3, 20, 5));
    let frame = panel_lines(&mut h).join("\n");
    let (filled, empty) = panel_markers(&frame);
    assert_eq!(filled, 3, "get_state 3/20 → 3 filled markers:\n{frame}");
    assert_eq!(empty, 17, "get_state 3/20 → 17 empty markers:\n{frame}");
}
