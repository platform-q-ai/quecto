//! Regression tests for reclaiming vertical conversation space previously used
//! by the main-pane workflow status box.

use super::tui_harness::*;
use crate::components::ansi::strip_ansi;
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

fn is_rule_row(line: &str) -> bool {
    let segment = line.rsplit_once("│ ").map(|(_, s)| s).unwrap_or(line);
    let t = segment.trim();
    !t.is_empty() && t.chars().all(|c| c == '─')
}

/// #1309: compact progress is framed by exactly two separator rules (above +
/// below). Not the old multi-line status box (#1246): no pills/hints.
fn compact_progress_framed(pane: &str) -> bool {
    let lines: Vec<&str> = pane.lines().collect();
    let Some(idx) = lines
        .iter()
        .position(|l| l.contains("Step 2/3") || (l.contains("1/3") && l.contains("Implement")))
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
async fn workflow_status_box_pills_and_hints_no_longer_render_in_main_pane() {
    let mut h = TuiHarness::sized(100, 30).await;
    h.event(Event::AgentStart);
    h.event(workflow_event());

    let pane = strip_ansi(&h.main_pane());
    assert!(
        compact_progress_framed(&pane),
        "compact progress must remain framed while pills/hints stay gone:\n{pane}"
    );
    assert!(
        !pane.contains("Ctrl+Shift+A") && !pane.contains("nudge:"),
        "phase-pills/hints multi-line widget must not return to the main pane:\n{pane}"
    );
    assert!(
        !pane.contains('○') && !pane.contains('●'),
        "phase-pill markers must not return to the main pane:\n{pane}"
    );
}

/// #1288 / #1309: active workflow surfaces live progress framed by separators.
#[tokio::test]
async fn active_workflow_shows_compact_progress_in_main_pane() {
    let mut h = TuiHarness::sized(100, 30).await;
    h.event(Event::AgentStart);
    h.event(workflow_event());

    let pane = strip_ansi(&h.main_pane());
    assert!(
        pane.contains("Step 2/3") || (pane.contains("1/3") && pane.contains("Implement")),
        "main-pane top must show live workflow progress for an active run:\n{pane}"
    );
    assert!(
        compact_progress_framed(&pane),
        "compact progress must have separator rules above and below:\n{pane}"
    );
    assert!(
        !pane.contains("Ctrl+Shift+A") && !pane.contains("nudge:"),
        "compact progress must not restore phase pills / shortcut hints:\n{pane}"
    );
}

/// #1288: progress must still render when no issue is bound (title alone is not enough).
#[tokio::test]
async fn active_workflow_without_issue_still_shows_main_pane_progress() {
    let mut h = TuiHarness::sized(100, 30).await;
    h.event(Event::AgentStart);
    h.event(Event::WorkflowState {
        agent_id: None,
        steps: vec![
            serde_json::json!({"index":1,"label":"Spec","phase":"red","done":true}),
            serde_json::json!({"index":2,"label":"Implement code","phase":"green","done":false}),
            serde_json::json!({"index":3,"label":"Review","phase":"review","done":false}),
        ],
        progress: serde_json::json!({"done":1,"total":3,"percent":33}),
        active_issue: None,
        mode: Some("active".to_string()),
        active_template: Some(serde_json::json!({"id":"feature","label":"Feature"})),
        available_templates: None,
    });

    let pane = strip_ansi(&h.main_pane());
    assert!(
        pane.contains("Step 2/3") || pane.contains("1/3"),
        "main-pane must show progress even without a bound issue:\n{pane}"
    );
    assert!(
        compact_progress_framed(&pane),
        "no-issue progress must still be framed by separator rules:\n{pane}"
    );
    assert!(
        !pane.contains("#882"),
        "no-issue run must not invent an issue number:\n{pane}"
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
async fn workflow_status_box_regression_test_fails_if_pills_hints_return() {
    let mut h = TuiHarness::sized(100, 30).await;
    h.event(workflow_event());

    let pane = strip_ansi(&h.main_pane());
    // Separator rules around the compact line are intentional (#1309).
    // Phase pills / shortcut hints from the multi-line widget must stay gone (#1246).
    assert!(
        compact_progress_framed(&pane),
        "compact progress must keep top/bottom separator rules:\n{pane}"
    );
    assert!(
        !pane.contains("Ctrl+Shift+A") && !pane.contains("nudge:"),
        "phase-pills/hints multi-line widget must not return to the main pane:\n{pane}"
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

    // With an active workflow the compact line adds rule + content + rule (#1309).
    minimum_width.event(workflow_event());
    let with_progress =
        minimum_width
            .app_mut()
            .render_main_pane_workflow(40, 40, tokio::time::Instant::now());
    assert_eq!(
        with_progress.len(),
        4,
        "title + top rule + compact line + bottom rule: {with_progress:?}"
    );
    let stripped: Vec<String> = with_progress.iter().map(|l| strip_ansi(l)).collect();
    assert!(
        is_rule_row(&stripped[1]) && is_rule_row(&stripped[3]),
        "progress must be framed by separator rules: {stripped:?}"
    );
    assert!(
        stripped[2].contains("Step 2/3")
            || (stripped[2].contains("1/3") && stripped[2].contains("Implement")),
        "middle line must be the compact progress content: {stripped:?}"
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
