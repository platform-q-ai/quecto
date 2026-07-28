use super::uds_execution_state::ExecutionState;
use crate::domain::agent::AgentProgressEvent;

#[test]
fn tool_events_report_live_and_recent_progress() {
    let mut state = ExecutionState::default();
    state.start_run();
    state.observe(&AgentProgressEvent::ToolStarted {
        tool_call_id: "call-1".into(),
        name: "bash".into(),
        arguments: "{}".into(),
    });
    let running = state.snapshot();
    assert_eq!(running.phase, "runningTool");
    assert_eq!(running.current_tool.as_ref().unwrap().name, "bash");
    assert_eq!(running.tools.started, 1);
    assert_eq!(running.progress.state, "active");

    state.observe(&AgentProgressEvent::ToolFinished {
        tool_call_id: "call-1".into(),
        name: "bash".into(),
        arguments: "{}".into(),
        result_content: "failed".into(),
        duration_ms: 1,
        is_error: true,
    });
    let completed = state.snapshot();
    assert_eq!(completed.phase, "thinking");
    assert!(completed.current_tool.is_none());
    assert_eq!(completed.tools.completed, 1);
    assert_eq!(completed.tools.failed, 1);
    assert_eq!(completed.progress.state, "advancing");
    assert_eq!(completed.progress.tool_calls_completed, 1);
    assert_eq!(completed.progress.tool_calls_failed, 1);
}

#[test]
fn new_run_resets_run_tool_totals_but_generation_remains_monotonic() {
    let mut state = ExecutionState::default();
    state.start_run();
    state.observe(&AgentProgressEvent::ToolStarted {
        tool_call_id: "call-1".into(),
        name: "read".into(),
        arguments: "{}".into(),
    });
    let prior_generation = state.snapshot().activity_generation;
    state.finish_run();
    state.start_run();
    let next = state.snapshot();
    assert!(next.activity_generation > prior_generation);
    assert_eq!(next.tools.started, 0);
    assert!(next.tools.used.is_empty());
    assert_eq!(next.phase, "thinking");
}
