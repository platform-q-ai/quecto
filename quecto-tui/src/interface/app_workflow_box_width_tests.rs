//! Width-alignment + current-step-context tests for the main-pane workflow box
//! (#882), driven through the headless render harness.
//!
//! The bordered box that frames the single-line workflow bar must span the same
//! body width as the tool-output background blocks (`render_tool_execution`),
//! and that alignment must survive a terminal resize. The boxed line must also
//! surface concise current-step context (step n/total, phase, label, issue).

use super::tui_harness::*;
use crate::infrastructure::client::Event;
use crate::interface::ansi::strip_ansi;
use crate::interface::utils::visible_width;

fn workflow_event() -> Event {
    Event::WorkflowState {
        agent_id: None,
        steps: vec![
            serde_json::json!({"index":1,"label":"Spec","phase":"red","done":true}),
            serde_json::json!({"index":2,"label":"Implement code","phase":"green","done":false}),
            serde_json::json!({"index":3,"label":"Review","phase":"review","done":false}),
        ],
        progress: serde_json::json!({"done":1,"total":3,"percent":33}),
        active_issue: Some(serde_json::json!({"number":882,"title":"box width"})),
        mode: Some("active".to_string()),
        active_template: None,
        available_templates: None,
    }
}

/// Visible widths of the workflow box's top border row (the line containing
/// `┌`), a representative tool-output box row, and the body-width contract
/// (`body_width()`) all three are pinned against. Returning `body_width()` (the
/// shared source of truth both renderers derive from) lets callers assert each
/// rendered width equals the contract, rather than comparing two co-derived
/// values to each other (a tautology). (#882 review)
fn box_tool_and_body_widths(h: &mut TuiHarness) -> (usize, usize, usize) {
    h.app_mut().active_chat_mut().start_tool(
        "c1".into(),
        "bash".into(),
        "{\"command\":\"echo hi\"}".into(),
    );
    h.app_mut()
        .active_chat_mut()
        .complete_tool("c1", "hi", false, Some(5));
    let body_w = h.app_mut().body_width();
    let frame: Vec<String> = h.full_frame().lines().map(|s| s.to_string()).collect();
    let box_w = frame
        .iter()
        .find(|l| l.contains('┌'))
        .map(|l| visible_width(l))
        .expect("workflow box top border should render");
    let tool_w = frame
        .iter()
        .find(|l| l.contains("$ echo hi"))
        .map(|l| visible_width(l))
        .expect("a tool-output box row should render");
    (box_w, tool_w, body_w)
}

/// Pin both rendered widths against the body-width contract: the panel split
/// reserves the left panel + divider, and the main pane (header prefix + the
/// `panel + divider + body` columns) totals the full terminal width. The box
/// border and tool block must each fill exactly the body column.
fn assert_widths_match_body(h: &mut TuiHarness, ctx: &str) {
    let (box_w, tool_w, body_w) = box_tool_and_body_widths(h);
    let (panel_w, divider_w, _) = h.app_mut().frame_split();
    let expected = panel_w + divider_w + body_w;
    assert_eq!(
        box_w, expected,
        "[{ctx}] workflow box must span the full main-pane width (= panel+divider+body)"
    );
    assert_eq!(
        tool_w, expected,
        "[{ctx}] tool-output block must span the full main-pane width (= panel+divider+body)"
    );
}

#[tokio::test]
async fn boxed_workflow_matches_tool_box_width() {
    let mut h = TuiHarness::sized(100, 30).await;
    h.event(workflow_event());
    assert_widths_match_body(&mut h, "width=100");
}

#[tokio::test]
async fn boxed_workflow_width_survives_resize() {
    let mut h = TuiHarness::sized(120, 30).await;
    h.event(workflow_event());
    assert_widths_match_body(&mut h, "width=120");

    // Resize narrower (as a SIGWINCH would) and re-check that both the box and
    // the tool block re-derive to the new body-width contract.
    h.app_mut().terminal.width = 72;
    h.app_mut().terminal.height = 30;
    assert_widths_match_body(&mut h, "width=72 (after resize)");
}

#[tokio::test]
async fn boxed_workflow_shows_current_step_context() {
    let mut h = TuiHarness::sized(120, 30).await;
    h.event(workflow_event());
    let pane = strip_ansi(&h.main_pane());
    assert!(
        pane.contains("Step 2/3"),
        "the boxed bar must show the current step number/total:\n{pane}"
    );
    assert!(
        pane.contains("GREEN"),
        "the boxed bar must show the current phase:\n{pane}"
    );
    assert!(
        pane.contains("Implement code"),
        "the boxed bar must show the current step label:\n{pane}"
    );
    assert!(
        pane.contains("#882"),
        "the boxed bar must show the active issue:\n{pane}"
    );
}
