//! Width-alignment + current-step-context tests for the main-pane workflow status
//! bar (#882/#947/#952), driven through the headless render harness.
//!
//! The status bar that frames the single-line workflow summary must be the SAME
//! width as the tool-output / message background blocks and left-aligned to the
//! same content column (one gutter space after the panel divider). This fails
//! against the post-#949 gutter-consuming render where the bar was one column
//! wider and ate the gutter. The status line must also surface concise
//! current-step context (step n/total, phase, label, issue) without wrapping or
//! overflowing.

use super::tui_harness::*;
use crate::components::ansi::strip_ansi;
use crate::components::utils::visible_width;
use crate::protocol::client::Event;

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

/// Visible width of the workflow status bar's top rule row and the body-width
/// contract. Tool calls now render as compact content-width rows rather than
/// filled panels, so this test pins the workflow bar's own full-width status-row
/// contract and shared left gutter without constraining the tool-call component.
fn workflow_and_body_widths(h: &mut TuiHarness) -> (usize, usize) {
    let body_w = h.app_mut().body_width();
    // Measure workflow box width from the actual rendered frame, not by calling
    // render_main_pane_workflow directly. This ensures we test the production
    // calculation in render() where main_box_width = width.
    let frame: Vec<String> = h.full_frame().lines().map(|s| s.to_string()).collect();
    let workflow_w = frame
        .iter()
        .find(|l| {
            strip_ansi(l).rsplit_once("│ ").is_some_and(|(_, segment)| {
                !segment.is_empty() && segment.chars().all(|c| c == '─')
            })
        })
        .map(|l| visible_width(l))
        .expect("workflow status top rule should render")
        .saturating_sub(h.app_mut().frame_split().0 + h.app_mut().frame_split().1);
    (workflow_w, body_w)
}

/// Pin the width contract after #952: the workflow status bar must equal the
/// body width and left-align to the content column (one gutter after the
/// divider). Tool-call rows are intentionally compact and are not used as a
/// width oracle.
fn assert_workflow_bar_aligned_with_tools(h: &mut TuiHarness, ctx: &str) {
    let (workflow_w, body_w) = workflow_and_body_widths(h);
    assert_eq!(
        workflow_w, body_w,
        "[{ctx}] workflow status bar width must equal the body width (not consume the gutter)"
    );
    // Verify left-edge alignment in the composed frame: the rule must appear
    // after the normal gutter (one space after the divider), matching tool rows.
    let frame: Vec<String> = h.full_frame().lines().map(|s| s.to_string()).collect();
    let rule_line = frame
        .iter()
        .find(|l| {
            strip_ansi(l).rsplit_once("│ ").is_some_and(|(_, segment)| {
                !segment.is_empty() && segment.chars().all(|c| c == '─')
            })
        })
        .expect("workflow status top rule should render in frame");
    assert!(
        strip_ansi(rule_line).contains("│ ─"),
        "[{ctx}] workflow status bar must start one column after the divider (aligned to tool/message content column), got:\n{rule_line}"
    );
}

#[tokio::test]
async fn workflow_status_bar_aligned_with_tool_output() {
    let mut h = TuiHarness::sized(100, 30).await;
    h.event(workflow_event());
    assert_workflow_bar_aligned_with_tools(&mut h, "width=100");
}

#[tokio::test]
async fn workflow_status_bar_alignment_survives_resize() {
    let mut h = TuiHarness::sized(120, 30).await;
    h.event(workflow_event());
    assert_workflow_bar_aligned_with_tools(&mut h, "width=120");

    // Resize narrower (as a SIGWINCH would) and re-check that both the workflow
    // bar and the tool block re-derive to the new body-width contract.
    h.app_mut().terminal.width = 72;
    h.app_mut().terminal.height = 30;
    assert_workflow_bar_aligned_with_tools(&mut h, "width=72 (after resize)");
}

#[tokio::test]
async fn workflow_status_bar_shows_current_step_context() {
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
async fn workflow_status_bar_truncates_at_narrow_width() {
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
    let (workflow_w, body_w) = workflow_and_body_widths(&mut h);
    assert_eq!(
        workflow_w, body_w,
        "narrow: workflow status bar width must equal body width"
    );
    let frame: Vec<String> = h.full_frame().lines().map(|s| s.to_string()).collect();
    let rule_line = frame
        .iter()
        .find(|l| {
            strip_ansi(l).rsplit_once("│ ").is_some_and(|(_, segment)| {
                !segment.is_empty() && segment.chars().all(|c| c == '─')
            })
        })
        .expect("workflow status top rule should render");
    assert!(
        strip_ansi(rule_line).contains("│ ─"),
        "narrow: workflow status bar must start after gutter, got:\n{rule_line}"
    );

    // Verify the content line survives truncation without overflowing or losing
    // its full-width status-row contract.
    let content_line = frame
        .iter()
        .find(|l| l.contains('░'))
        .expect("workflow content line should render");
    let stripped = strip_ansi(content_line);
    let divider = stripped
        .find('│')
        .expect("composed frame should include divider");
    let status_portion = &stripped[divider + '│'.len_utf8() + 1..];
    assert!(
        status_portion.starts_with(' '),
        "narrow: workflow status content should preserve left padding, got:\n{status_portion}"
    );
    assert_eq!(
        visible_width(status_portion),
        body_w,
        "narrow: workflow status content width must equal body width"
    );
    assert_eq!(
        visible_width(content_line),
        h.terminal_width(),
        "narrow: composed workflow status row must not overflow the terminal"
    );
}
