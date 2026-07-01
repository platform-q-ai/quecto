//! Width-alignment + current-step-context tests for the main-pane workflow box
//! (#882/#947/#952), driven through the headless render harness.
//!
//! The bordered box that frames the single-line workflow bar must be the SAME
//! width as the tool-output / message boxes and left-aligned to the same
//! content column (one gutter space after the panel divider). This fails
//! against the post-#949 gutter-consuming render where the box was one column
//! wider and ate the gutter. The boxed line must also surface concise
//! current-step context (step n/total, phase, label, issue) without wrapping
//! or overflowing.

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
/// After #952 the workflow box must equal the body/tool width and start at the
/// same content column (one gutter space after the divider).
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
            .render_main_pane_workflow(body_w, body_w, now)
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

/// Pin the width contract after #952: the workflow box must equal the body/tool
/// width and left-align to the same content column (one gutter after the
/// divider). This fails against the post-#949 gutter-consuming render where the
/// box was one column wider.
fn assert_workflow_box_aligned_with_tools(h: &mut TuiHarness, ctx: &str) {
    let (box_w, tool_w, body_w) = box_tool_and_body_widths(h);
    assert_eq!(
        box_w, body_w,
        "[{ctx}] workflow box width must equal the body width (not consume the gutter)"
    );
    assert_eq!(
        box_w, tool_w,
        "[{ctx}] workflow box width must equal the tool-output width"
    );
    // Verify left-edge alignment in the composed frame: the box's left border
    // must appear after the normal gutter (one space after the divider).
    let frame: Vec<String> = h.full_frame().lines().map(|s| s.to_string()).collect();
    let border_line = frame
        .iter()
        .find(|l| l.contains('┌'))
        .expect("workflow box top border should render in frame");
    assert!(
        border_line.contains("│ ┌"),
        "[{ctx}] workflow box must start one column after the divider (aligned to tool/message content column), got:\n{border_line}"
    );
}

#[tokio::test]
async fn boxed_workflow_aligned_with_tool_output() {
    let mut h = TuiHarness::sized(100, 30).await;
    h.event(workflow_event());
    assert_workflow_box_aligned_with_tools(&mut h, "width=100");
}

#[tokio::test]
async fn boxed_workflow_alignment_survives_resize() {
    let mut h = TuiHarness::sized(120, 30).await;
    h.event(workflow_event());
    assert_workflow_box_aligned_with_tools(&mut h, "width=120");

    // Resize narrower (as a SIGWINCH would) and re-check that both the box and
    // the tool block re-derive to the new body-width contract.
    h.app_mut().terminal.width = 72;
    h.app_mut().terminal.height = 30;
    assert_workflow_box_aligned_with_tools(&mut h, "width=72 (after resize)");
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

#[tokio::test]
async fn boxed_workflow_truncates_at_narrow_width() {
    let mut h = TuiHarness::sized(60, 30).await;
    h.event(Event::WorkflowState {
        agent_id: None,
        steps: vec![
            serde_json::json!({"index":1,"label":"Very long step name that should truncate","phase":"green","done":false}),
        ],
        progress: serde_json::json!({"done":0,"total":1,"percent":0}),
        active_issue: Some(serde_json::json!({"number":952,"title":"workflow box alignment fix"})),
        mode: Some("active".to_string()),
        active_template: None,
        available_templates: None,
    });
    let (box_w, tool_w, body_w) = box_tool_and_body_widths(&mut h);
    assert_eq!(
        box_w, body_w,
        "narrow: workflow box width must equal body width"
    );
    assert_eq!(
        box_w, tool_w,
        "narrow: workflow box width must equal tool width"
    );
    let frame: Vec<String> = h.full_frame().lines().map(|s| s.to_string()).collect();
    let border_line = frame
        .iter()
        .find(|l| l.contains('┌'))
        .expect("workflow box top border should render");
    assert!(
        border_line.contains("│ ┌"),
        "narrow: workflow box must start after gutter, got:\n{border_line}"
    );
    let content_line = frame
        .iter()
        .find(|l| l.contains('│') && l.contains('░'))
        .expect("workflow content line should render");

    // The full composed line should not exceed the terminal width (60).
    let line_width = visible_width(content_line);
    assert_eq!(
        line_width, 60,
        "narrow: workflow content line width ({}) must equal terminal width (60)",
        line_width
    );
}
