use super::tui_harness::TuiHarness;
use super::*;
use crate::components::workflow_bar;

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

#[tokio::test]
async fn main_pane_compact_line_reflects_live_auto_continue_state() {
    // #897 AC2: live auto-continue must drive the compact line after rebuild.
    let mut h = harness().await;
    let wf = serde_json::json!({
        "steps": [{"index": 0, "label": "Build it", "phase": "build", "done": false}],
        "progress": {"done": 0, "total": 1},
        "activeIssue": {"number": 7, "title": "thing"}
    });
    h.app_mut().active_conn_mut().master_session.workflow_bar =
        workflow_bar::parse_workflow_event(&wf);
    let now = tokio::time::Instant::now();
    let render = |a: &App| -> String {
        a.render_main_pane_workflow(120, 120, now)
            .iter()
            .map(|l| super::app_methods::strip_ansi(l))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert!(
        render(h.app_mut()).contains("auto:off"),
        "{}",
        render(h.app_mut())
    );
    h.app_mut().handle_response(
        Some("workflow-auto".into()),
        "set_workflow_automation".into(),
        true,
        Some(serde_json::json!({"automation": {"autoContinue": true}})),
        None,
    );
    assert!(
        render(h.app_mut()).contains("auto:on"),
        "{}",
        render(h.app_mut())
    );
    h.app_mut().active_conn_mut().master_session.workflow_bar =
        workflow_bar::parse_workflow_event(&wf);
    h.app_mut().mirror_automation_to_bar();
    assert!(
        render(h.app_mut()).contains("auto:on"),
        "workflow_state rebuild must preserve live auto-continue: {}",
        render(h.app_mut())
    );
}
