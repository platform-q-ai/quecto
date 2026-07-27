//! Behavioural tests for #901: sub-agent workflow indicators must (1) appear
//! AS SOON AS a workflow/template is selected — including the `0/N` state before
//! any step completes — and (2) be STICKY: once shown, stay visible and only
//! advance on live events, cleared ONLY by a genuine workflow end/reset, never
//! by a transient/empty intermediate `workflow_state` event.
//!
//! Driven through the headless render harness and the real per-agent routing
//! (`route_subagent_event`), so they exercise the same path the live UDS stream
//! does. They preserve #869 (get_state populate + live updates) and #840
//! (per-agent routing).

use super::tui_harness::*;
use crate::components::ansi::strip_ansi;
use crate::protocol::client::Event;

/// A `workflow_state` event for `agent` with `total` steps, `done` of them
/// complete, no active issue and `mode == "active"` — i.e. a freshly selected
/// workflow at `done/total`.
fn workflow_count(agent: &str, done: u32, total: u32) -> Event {
    let steps: Vec<serde_json::Value> = (1..=total)
        .map(|i| {
            serde_json::json!({
                "index": i,
                "label": format!("step {i}"),
                "phase": "red",
                "done": i <= done,
            })
        })
        .collect();
    Event::WorkflowState {
        agent_id: Some(agent.to_string()),
        steps,
        progress: serde_json::json!({"done": done, "total": total}),
        active_issue: None,
        mode: Some("active".to_string()),
        active_template: None,
        available_templates: None,
    }
}

/// A transient/empty live `workflow_state` event: `0/0`, no issue, no steps,
/// `mode == "active"` (NOT an explicit end/reset). Emitted by the kernel around
/// a transition/nudge (#899) — must NOT blank an already-visible workflow.
fn workflow_empty(agent: &str) -> Event {
    Event::WorkflowState {
        agent_id: Some(agent.to_string()),
        steps: vec![],
        progress: serde_json::json!({"done": 0, "total": 0}),
        active_issue: None,
        mode: Some("active".to_string()),
        active_template: None,
        available_templates: None,
    }
}

/// A genuine workflow END event on the forwarded descendant path: steps and
/// templates have been dropped (`canonical_workflow_forward`) and progress has
/// reset to `0/0`, but `mode` is the real terminal value the kernel emits
/// (`"complete"`, per `WorkflowMode::wire_str`), so the indicators SHOULD clear
/// rather than stick. Distinct from a `#899` transient, which is `"active"`/
/// `"selecting_template"`.
fn workflow_reset(agent: &str) -> Event {
    Event::WorkflowState {
        agent_id: Some(agent.to_string()),
        steps: vec![],
        progress: serde_json::json!({"done": 0, "total": 0}),
        active_issue: None,
        mode: Some("complete".to_string()),
        active_template: None,
        available_templates: None,
    }
}

/// Any per-step workflow glyph (filled or pending) in the LEFT panel cells.
fn has_panel_cells(frame: &str) -> bool {
    frame.contains('▰') || frame.contains('▱')
}

async fn harness_with_worker() -> TuiHarness {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    // Track the worker so `route_subagent_event` is not dropped as stale.
    h.event(subagents_changed(vec![subagent(
        "worker",
        "running",
        Some(("active", 0, 18)),
    )]));
    h
}

// (a) A just-selected 0/N workflow renders BOTH indicators immediately.
#[tokio::test]
async fn just_selected_zero_of_n_renders_both_indicators() {
    let mut h = harness_with_worker().await;
    h.app_mut()
        .route_subagent_event("worker", workflow_count("worker", 0, 18));
    h.app_mut().select_agent(Some("worker"));
    let frame = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(
        has_panel_cells(&frame),
        "a just-selected 0/18 workflow must render the LEFT panel cells:\n{frame}"
    );
    assert!(
        frame.contains("worker · running") && !frame.contains("Step 1/18"),
        "a just-selected 0/18 workflow must keep only the compact main-pane title:\n{frame}"
    );
}

// (b) An intermediate empty live event between two valid ones keeps both
//     indicators visible (no flicker).
#[tokio::test]
async fn transient_empty_event_does_not_blank_visible_workflow() {
    let mut h = harness_with_worker().await;
    h.app_mut()
        .route_subagent_event("worker", workflow_count("worker", 3, 18));
    h.app_mut().select_agent(Some("worker"));
    // A transient empty event arrives around a transition/nudge.
    h.app_mut()
        .route_subagent_event("worker", workflow_empty("worker"));
    let frame = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(
        has_panel_cells(&frame),
        "a transient empty workflow_state must NOT blank the LEFT panel cells:\n{frame}"
    );
    assert!(
        frame.contains("worker · running") && !frame.contains("Step 4/18"),
        "a transient empty workflow_state must keep only the compact main-pane title:\n{frame}"
    );
    // A subsequent real event still advances progress.
    h.app_mut()
        .route_subagent_event("worker", workflow_count("worker", 5, 18));
    let frame = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(
        frame.contains("worker · running") && !frame.contains("Step 6/18"),
        "a real workflow_state after a transient one still keeps the main-pane box removed:\n{frame}"
    );
}

// (c) A genuine reset/end event DOES clear the indicators.
#[tokio::test]
async fn genuine_reset_event_clears_indicators() {
    let mut h = harness_with_worker().await;
    h.app_mut()
        .route_subagent_event("worker", workflow_count("worker", 3, 18));
    h.app_mut().select_agent(Some("worker"));
    h.app_mut()
        .route_subagent_event("worker", workflow_reset("worker"));
    let frame = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(
        !frame.contains("Step 4/18"),
        "a genuine reset must clear the main-pane bar:\n{frame}"
    );
    assert!(
        !has_panel_cells(&frame),
        "a genuine reset must clear the LEFT panel cells:\n{frame}"
    );
}
