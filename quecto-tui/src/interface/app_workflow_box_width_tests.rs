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

/// Width of the workflow box's top border row (the line containing `┌`) and of a
/// representative tool-output box row, both ANSI-stripped, in the rendered frame.
fn box_and_tool_widths(h: &mut TuiHarness) -> (usize, usize) {
    h.app_mut().active_chat_mut().start_tool(
        "c1".into(),
        "bash".into(),
        "{\"command\":\"echo hi\"}".into(),
    );
    h.app_mut()
        .active_chat_mut()
        .complete_tool("c1", "hi", false, Some(5));
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
    (box_w, tool_w)
}

#[tokio::test]
async fn boxed_workflow_matches_tool_box_width() {
    let mut h = TuiHarness::sized(100, 30).await;
    h.event(workflow_event());
    let (box_w, tool_w) = box_and_tool_widths(&mut h);
    assert_eq!(
        box_w, tool_w,
        "the workflow box must span the same width as the tool-output blocks"
    );
}

#[tokio::test]
async fn boxed_workflow_width_survives_resize() {
    let mut h = TuiHarness::sized(120, 30).await;
    h.event(workflow_event());
    let (box_w, tool_w) = box_and_tool_widths(&mut h);
    assert_eq!(box_w, tool_w, "alignment must hold at the initial width");

    // Resize narrower (as a SIGWINCH would) and re-check.
    h.app_mut().terminal.width = 72;
    h.app_mut().terminal.height = 30;
    let (box_w, tool_w) = box_and_tool_widths(&mut h);
    assert_eq!(
        box_w, tool_w,
        "alignment must hold after a terminal resize event"
    );
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
