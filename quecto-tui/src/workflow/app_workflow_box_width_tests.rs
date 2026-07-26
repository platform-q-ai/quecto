//! Regression tests for reclaiming vertical conversation space previously used
//! by the main-pane workflow status box.

use super::tui_harness::*;
use crate::components::ansi::strip_ansi;
use crate::components::chat::Chat;
use crate::protocol::client::Event;
use crate::shell::app::App;

fn active_viewport_height(app: &mut App) -> Option<usize> {
    let Chat {
        viewport_height, ..
    } = app.active_chat_mut();
    *viewport_height
}

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
    let _ = without_workflow.full_frame();
    let baseline_height = active_viewport_height(without_workflow.app_mut());

    let mut with_workflow = TuiHarness::sized(100, 30).await;
    with_workflow.event(Event::AgentStart);
    with_workflow.event(workflow_event());
    let _ = with_workflow.full_frame();
    let workflow_height = active_viewport_height(with_workflow.app_mut());

    assert_eq!(
        workflow_height, baseline_height,
        "an active workflow must not reduce the conversation viewport height"
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
    for terminal_height in [12, 13] {
        let mut without_workflow = TuiHarness::sized(100, terminal_height).await;
        without_workflow.event(Event::AgentStart);
        let _ = without_workflow.full_frame();
        let baseline_height = active_viewport_height(without_workflow.app_mut());

        let mut with_workflow = TuiHarness::sized(100, terminal_height).await;
        with_workflow.event(Event::AgentStart);
        with_workflow.event(workflow_event());
        let _ = with_workflow.full_frame();
        let workflow_height = active_viewport_height(with_workflow.app_mut());

        assert_eq!(
            workflow_height, baseline_height,
            "active workflow must not reduce chat height at terminal height {terminal_height}"
        );
    }
}
