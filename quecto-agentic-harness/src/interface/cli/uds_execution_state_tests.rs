use super::uds_execution_state::ExecutionState;
use crate::domain::agent::AgentProgressEvent;

fn started(id: &str, name: &str) -> AgentProgressEvent {
    AgentProgressEvent::ToolStarted {
        tool_call_id: id.into(),
        name: name.into(),
        arguments: "{}".into(),
    }
}
fn finished(id: &str, name: &str, is_error: bool) -> AgentProgressEvent {
    AgentProgressEvent::ToolFinished {
        tool_call_id: id.into(),
        name: name.into(),
        arguments: "{}".into(),
        result_content: String::new(),
        duration_ms: 1,
        is_error,
    }
}

#[test]
fn tool_events_report_live_and_recent_progress() {
    let mut state = ExecutionState::default();
    state.start_run();
    state.observe(&started("call-1", "bash"));
    let running = state.snapshot();
    assert_eq!(running.phase, "runningTool");
    assert_eq!(running.current_tool.as_ref().unwrap().name, "bash");
    assert_eq!(running.tools.started, 1);
    assert_eq!(running.progress.state, "active");
    assert!(humantime::parse_rfc3339(&running.last_activity_at).is_ok());
    assert!(humantime::parse_rfc3339(&running.current_tool.unwrap().started_at).is_ok());

    state.observe(&finished("call-1", "bash", true));
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
fn finishing_one_overlapping_tool_preserves_the_other() {
    let mut state = ExecutionState::default();
    state.start_run();
    state.observe(&started("call-1", "read"));
    state.observe(&started("call-2", "bash"));
    state.observe(&finished("call-1", "read", false));
    let snapshot = state.snapshot();
    assert_eq!(snapshot.phase, "runningTool");
    assert_eq!(snapshot.current_tool.unwrap().call_id, "call-2");
}

#[test]
fn thinking_activity_does_not_replace_latest_progress_time() {
    let mut state = ExecutionState::default();
    state.start_run();
    state.observe(&started("call-1", "read"));
    state.observe(&finished("call-1", "read", false));
    let before = state.snapshot().progress.last_progress_seconds_ago;
    state.observe(&AgentProgressEvent::Thinking {
        context_tokens: 10,
        max_context_tokens: 100,
        provider: "test".into(),
        model: "test-model".into(),
    });
    let after = state.snapshot().progress.last_progress_seconds_ago;
    assert!(after >= before);
}

#[test]
fn duplicate_or_unknown_finishes_do_not_inflate_progress() {
    let mut state = ExecutionState::default();
    state.start_run();
    state.observe(&started("call-1", "read"));
    state.observe(&finished("unknown", "read", true));
    state.observe(&finished("call-1", "read", false));
    state.observe(&finished("call-1", "read", true));
    let snapshot = state.snapshot();
    assert_eq!(snapshot.tools.completed, 1);
    assert_eq!(snapshot.tools.failed, 0);
    assert_eq!(snapshot.progress.tool_calls_completed, 1);
}

#[test]
fn conversation_changes_reconcile_pruning_and_final_append_counts() {
    let mut state = ExecutionState::default();
    state.set_message_count(8);
    state.observe(&AgentProgressEvent::ConversationChanged {
        messages: Vec::from([
            crate::domain::message::Message::user("one"),
            crate::domain::message::Message::assistant("two", vec![]),
        ])
        .into(),
    });
    assert_eq!(state.message_count(), 2);
}

#[test]
fn message_count_can_be_reconciled_after_lifecycle_changes() {
    let mut state = ExecutionState::default();
    state.set_message_count(8);
    state.set_message_count(2);
    assert_eq!(state.message_count(), 2);
    state.set_message_count(0);
    assert_eq!(state.message_count(), 0);
}

#[test]
fn new_run_resets_run_tool_totals_but_generation_remains_monotonic() {
    let mut state = ExecutionState::default();
    state.start_run();
    state.observe(&started("call-1", "read"));
    let prior_generation = state.snapshot().activity_generation;
    state.finish_run();
    state.start_run();
    let next = state.snapshot();
    assert!(next.activity_generation > prior_generation);
    assert_eq!(next.tools.started, 0);
    assert!(next.tools.used.is_empty());
    assert_eq!(next.phase, "thinking");
}

#[test]
fn visible_generation_is_owned_and_monotonic_across_activity_and_idle() {
    let mut state = ExecutionState::default();
    let initial = state.observe_visible_revisions(1, 0);
    state.start_run();
    let streaming = state.observe_visible_revisions(2, 0);
    state.finish_run();
    let idle = state.observe_visible_revisions(3, 0);

    assert!(streaming > initial);
    assert!(idle > streaming);
}

#[test]
fn visible_generation_advances_once_per_new_component_revision() {
    let mut state = ExecutionState::default();
    let initial = state.observe_visible_revisions(7, 3);
    let unchanged = state.observe_visible_revisions(7, 3);
    let workflow_changed = state.observe_visible_revisions(7, 4);
    let session_changed = state.observe_visible_revisions(8, 4);

    assert_eq!(unchanged, initial);
    assert_eq!(workflow_changed, initial + 1);
    assert_eq!(session_changed, workflow_changed + 1);
}
