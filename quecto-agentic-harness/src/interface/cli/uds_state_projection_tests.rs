use super::protocol::SessionState;
use super::uds_execution_state::{ExecutionSnapshot, ProgressSummary, ToolSummary};
use super::uds_state_projection::{slim_state_projection, slim_state_response_data};

fn state_with_execution(activity_generation: u64, progress_state: &str) -> SessionState {
    SessionState {
        model: "mock".into(),
        generation: 1,
        is_streaming: true,
        session_key: "s".into(),
        message_count: 0,
        pending_message_count: 0,
        max_context_tokens: 0,
        effort: None,
        effort_levels: vec![],
        workflow: None,
        execution: Some(ExecutionSnapshot {
            phase: "runningTool".into(),
            activity_generation,
            last_activity_at: "now".into(),
            last_activity_seconds_ago: 0,
            current_tool: None,
            tools: ToolSummary::default(),
            progress: ProgressSummary {
                state: progress_state.into(),
                reason: "tool activity".into(),
                ..Default::default()
            },
        }),
        sync: 0,
    }
}

#[test]
fn progress_uses_state_key_not_verdict() {
    let data = slim_state_projection(&state_with_execution(7, "advancing"));
    assert_eq!(data["progress"]["state"], "advancing");
    assert_eq!(data["progress"]["reason"], "tool activity");
    assert!(
        data["progress"].get("verdict").is_none(),
        "progress must not expose verdict: {data}"
    );
}

#[test]
fn idle_generation_uses_session_cursor_even_when_execution_snapshot_exists() {
    let mut state = state_with_execution(8, "quiet");
    state.is_streaming = false;
    state.generation = 3;
    let data = slim_state_response_data(&state, Some(8));
    assert_eq!(data["generation"], 3);
    assert!(
        data.get("unchanged").is_none(),
        "idle get_state must compare since against session generation, not stale execution: {data}"
    );
}

#[test]
fn generation_uses_activity_generation_even_when_projection_fields_do_not_change() {
    let prior = state_with_execution(7, "advancing");
    let later = state_with_execution(8, "advancing");
    let data = slim_state_response_data(
        &later,
        Some(
            slim_state_projection(&prior)["generation"]
                .as_u64()
                .unwrap(),
        ),
    );
    assert_eq!(data["generation"], 8);
    assert!(
        data.get("unchanged").is_none(),
        "activity generation change must not be hidden as unchanged: {data}"
    );
}

#[test]
fn streaming_generation_includes_live_workflow_revision_overlay() {
    let mut prior = state_with_execution(7, "advancing");
    prior.generation = 7;
    prior.workflow = Some(serde_json::json!({
        "activeTemplate": { "id": "bugfix" },
        "currentStep": { "index": 0, "key": "red", "label": "RED", "phase": "test", "done": false }
    }));

    let mut later = prior.clone();
    later.generation = 9;
    later.workflow = Some(serde_json::json!({
        "activeTemplate": { "id": "bugfix" },
        "currentStep": { "index": 1, "key": "green", "label": "GREEN", "phase": "fix", "done": false }
    }));

    let data = slim_state_response_data(
        &later,
        Some(
            slim_state_projection(&prior)["generation"]
                .as_u64()
                .unwrap(),
        ),
    );

    assert_eq!(data["generation"], 9);
    assert_eq!(data["workflow"]["currentStep"]["key"], "green");
    assert!(
        data.get("unchanged").is_none(),
        "streaming workflow-only changes must bump the emitted cursor: {data}"
    );
}
