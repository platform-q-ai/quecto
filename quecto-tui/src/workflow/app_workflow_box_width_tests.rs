//! Regression tests for reclaiming vertical conversation space previously used
//! by the main-pane workflow status box.

use super::tui_harness::*;
use crate::components::ansi::strip_ansi;
use crate::components::workflow_bar;
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

#[tokio::test]
async fn workflow_status_box_no_longer_renders_in_main_pane() {
    let mut h = TuiHarness::sized(100, 30).await;
    h.event(workflow_event());

    let pane = strip_ansi(&h.main_pane());
    assert!(
        !pane.lines().any(|line| line.chars().all(|c| c == '─')),
        "the old workflow status rule rows must not render:\n{pane}"
    );
    assert!(
        !pane.contains("Step 2/3") && !pane.contains("Implement code"),
        "the old workflow status content line must not render above the chat:\n{pane}"
    );
}

#[tokio::test]
async fn workflow_status_box_removal_keeps_workflow_viewport_at_max_height() {
    let mut without_workflow = TuiHarness::sized(100, 30).await;
    without_workflow.event(Event::AgentStart);
    without_workflow.add_user_message("VISIBLE_BASELINE_TAIL");
    let baseline = without_workflow.main_pane();

    let mut with_workflow = TuiHarness::sized(100, 30).await;
    with_workflow.event(Event::AgentStart);
    with_workflow.event(workflow_event());
    with_workflow.add_user_message("VISIBLE_WORKFLOW_TAIL");
    let workflow = with_workflow.main_pane();

    assert!(
        baseline.contains("VISIBLE_BASELINE_TAIL"),
        "baseline chat tail must be visible:\n{baseline}"
    );
    assert!(
        workflow.contains("VISIBLE_WORKFLOW_TAIL"),
        "an active workflow must not reduce the conversation viewport enough to hide the chat tail:\n{workflow}"
    );
}

#[tokio::test]
async fn chat_tail_sits_directly_above_input_bar() {
    let mut h = TuiHarness::sized(100, 30).await;
    h.add_user_message("NO_GAP_TAIL");

    let frame = strip_ansi(&h.full_frame());
    let lines: Vec<&str> = frame.lines().collect();
    let tail = lines
        .iter()
        .position(|line| line.contains("NO_GAP_TAIL"))
        .unwrap_or_else(|| panic!("chat tail should render:\n{frame}"));
    let input = lines
        .iter()
        .enumerate()
        .skip(tail + 1)
        .find(|(_, line)| line.contains(" > ") && line.contains('─'))
        .map(|(idx, _)| idx)
        .unwrap_or_else(|| panic!("input bar should render below chat:\n{frame}"));

    assert_eq!(
        input,
        tail + 1,
        "chat tail must sit directly above the input bar without reserved blank gap:\n{frame}"
    );
}

#[tokio::test]
async fn main_pane_title_reflects_live_auto_continue_state() {
    let mut h = TuiHarness::new().await;
    let wf = serde_json::json!({
        "steps": [{"index": 0, "label": "Build it", "phase": "build", "done": false}],
        "progress": {"done": 0, "total": 1},
        "activeIssue": {"number": 7, "title": "thing"}
    });
    h.app_mut().master_session.workflow_bar = workflow_bar::parse_workflow_event(&wf);
    let now = tokio::time::Instant::now();
    let render = |h: &mut TuiHarness| -> String {
        h.app_mut()
            .render_main_pane_workflow(120, 120, now)
            .iter()
            .map(|l| strip_ansi(l))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert!(render(&mut h).contains("auto:off"), "{}", render(&mut h));
    h.app_mut().handle_response(
        Some("workflow-auto".into()),
        "set_workflow_automation".into(),
        true,
        Some(serde_json::json!({"automation": {"autoContinue": true}})),
        None,
    );
    assert!(render(&mut h).contains("auto:on"), "{}", render(&mut h));
    h.app_mut().master_session.workflow_bar = workflow_bar::parse_workflow_event(&wf);
    h.app_mut().mirror_automation_to_bar();
    assert!(
        render(&mut h).contains("auto:on"),
        "workflow_state rebuild must preserve workflow title auto-continue state: {}",
        render(&mut h)
    );
}

#[tokio::test]
async fn workflow_title_respects_minimum_width_boundary() {
    let mut too_narrow = TuiHarness::sized(3, 30).await;
    assert!(
        too_narrow
            .app_mut()
            .render_main_pane_workflow(3, 3, tokio::time::Instant::now())
            .is_empty(),
        "width 3 must remain too narrow to render title chrome"
    );

    let mut minimum_width = TuiHarness::sized(4, 30).await;
    assert_eq!(
        minimum_width
            .app_mut()
            .render_main_pane_workflow(4, 4, tokio::time::Instant::now())
            .len(),
        1,
        "width 4 is the first width that renders the title chrome"
    );
}

#[tokio::test]
async fn workflow_does_not_reduce_viewport_at_near_threshold_heights() {
    for terminal_height in [13, 14] {
        let mut without_workflow = TuiHarness::sized(100, terminal_height).await;
        without_workflow.event(Event::AgentStart);
        without_workflow.add_user_message("BASELINE_NEAR_THRESHOLD_TAIL");
        let baseline = without_workflow.main_pane();

        let mut with_workflow = TuiHarness::sized(100, terminal_height).await;
        with_workflow.event(Event::AgentStart);
        with_workflow.event(workflow_event());
        with_workflow.add_user_message("WORKFLOW_NEAR_THRESHOLD_TAIL");
        let workflow = with_workflow.main_pane();

        assert!(
            baseline.contains("BASELINE_NEAR_THRESHOLD_TAIL"),
            "baseline chat tail must be visible at terminal height {terminal_height}:\n{baseline}"
        );
        assert!(
            workflow.contains("WORKFLOW_NEAR_THRESHOLD_TAIL"),
            "active workflow must not reduce chat visibility at terminal height {terminal_height}:\n{workflow}"
        );
    }
}
