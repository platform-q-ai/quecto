//! Width-alignment + current-step-context tests for the main-pane workflow box
//! (#882/#947), driven through the headless render harness.
//!
//! The bordered box that frames the single-line workflow bar must span the full
//! main-panel content width. It is intentionally wider than the inset
//! tool-output/background blocks, and that relationship must survive terminal
//! resize. The boxed line must also surface concise current-step context (step
//! n/total, phase, label, issue) without wrapping or overflowing.

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
/// `┌`), a representative tool-output box row, and the body-width contract.
/// The workflow box should use the wider main-panel contract while tool output
/// remains inset to the body/tool contract. (#947)
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
    let box_w = {
        let now = tokio::time::Instant::now();
        h.app_mut()
            .render_main_pane_workflow(body_w, now)
            .iter()
            .find(|l| l.contains('┌'))
            .map(|l| visible_width(l))
            .expect("workflow box top border should render")
    };
    let frame: Vec<String> = h.full_frame().lines().map(|s| s.to_string()).collect();
    let tool_w = frame
        .iter()
        .find(|l| l.contains("$ echo hi"))
        .map(|l| visible_width(l))
        .expect("a tool-output box row should render")
        .saturating_sub(h.app_mut().frame_split().0 + h.app_mut().frame_split().1);
    (box_w, tool_w, body_w)
}

/// Pin the new width contract: after the persistent panel is prefixed, the
/// workflow box consumes the full terminal row, while tool output remains inset
/// to the body column. This fails against the old body/tool-box alignment.
fn assert_workflow_box_is_full_width_and_wider_than_tools(h: &mut TuiHarness, ctx: &str) {
    let (box_w, tool_w, body_w) = box_tool_and_body_widths(h);
    let expected_box_w = body_w + 1;
    assert_eq!(
        box_w, expected_box_w,
        "[{ctx}] workflow box border must span the full main-panel content width"
    );
    assert_eq!(
        tool_w, body_w,
        "[{ctx}] tool-output block keeps the existing body/tool alignment"
    );
    assert!(
        box_w > tool_w,
        "[{ctx}] workflow box must be visibly wider than tool output (box={box_w}, tool={tool_w})"
    );
}

#[tokio::test]
async fn boxed_workflow_spans_full_width_and_is_wider_than_tool_output() {
    let mut h = TuiHarness::sized(100, 30).await;
    h.event(workflow_event());
    assert_workflow_box_is_full_width_and_wider_than_tools(&mut h, "width=100");
}

#[tokio::test]
async fn boxed_workflow_width_survives_resize() {
    let mut h = TuiHarness::sized(120, 30).await;
    h.event(workflow_event());
    assert_workflow_box_is_full_width_and_wider_than_tools(&mut h, "width=120");

    // Resize narrower (as a SIGWINCH would) and re-check that both the box and
    // the tool block re-derive to the new body-width contract.
    h.app_mut().terminal.width = 72;
    h.app_mut().terminal.height = 30;
    assert_workflow_box_is_full_width_and_wider_than_tools(&mut h, "width=72 (after resize)");
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
