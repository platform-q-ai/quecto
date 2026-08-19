use super::protocol::SessionState;
use super::uds_execution_state::{ExecutionSnapshot, ProgressSummary, ToolSummary};
use super::uds_state_projection::{
    slim_progress, slim_state_projection, slim_state_response_data, slim_workflow,
};

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
fn projection_uses_owned_visible_generation_when_idle() {
    let mut state = state_with_execution(8, "quiet");
    state.is_streaming = false;
    state.generation = 3;
    let data = slim_state_response_data(&state, Some(8));
    assert_eq!(data["generation"], 3);
    assert!(
        data.get("unchanged").is_none(),
        "projection must use the cursor already owned by supervision state: {data}"
    );
}

#[test]
fn projection_does_not_recompose_generation_from_execution_revision() {
    let mut state = state_with_execution(8, "advancing");
    state.generation = 42;

    let data = slim_state_projection(&state);

    assert_eq!(data["generation"], 42);
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

#[test]
fn streaming_workflow_overlay_advances_cursor_when_activity_generation_is_ahead() {
    let mut prior = state_with_execution(100, "advancing");
    prior.generation = 2; // session generation 1 + workflow revision 1
    prior.workflow = Some(serde_json::json!({
        "activeTemplate": { "id": "bugfix" },
        "currentStep": { "index": 0, "key": "red", "label": "RED", "phase": "test", "done": false }
    }));

    let mut later = prior.clone();
    later.generation = 3; // same session generation + workflow revision 2
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

    assert_eq!(data["workflow"]["currentStep"]["key"], "green");
    assert!(
        data["generation"].as_u64().unwrap()
            > slim_state_projection(&prior)["generation"]
                .as_u64()
                .unwrap(),
        "workflow-only streaming changes must advance the emitted cursor even when execution activity is ahead: {data}"
    );
    assert!(
        data.get("unchanged").is_none(),
        "changed workflow step must not collapse to unchanged under the prior cursor: {data}"
    );
}

#[test]
fn generation_does_not_regress_when_streaming_snapshot_becomes_idle() {
    let mut streaming = state_with_execution(100, "advancing");
    streaming.generation = 3;
    let streaming_generation = slim_state_projection(&streaming)["generation"]
        .as_u64()
        .unwrap();

    let mut idle = streaming.clone();
    idle.is_streaming = false;
    idle.generation = streaming_generation.saturating_add(1);
    let idle_generation = slim_state_projection(&idle)["generation"].as_u64().unwrap();

    assert!(
        idle_generation > streaming_generation,
        "visible get_state generation must be monotonic across streaming->idle: streaming={streaming_generation}, idle={idle_generation}"
    );
}

#[test]
fn slim_workflow_accepts_snake_case_template_and_step() {
    let workflow = slim_workflow(&serde_json::json!({
        "active_template": { "id": "bugfix", "label": "Bugfix" },
        "current_step": {
            "index": 2,
            "key": "sweep",
            "label": "Sweep",
            "phase": "green",
            "done": true,
            "guidance": "hidden"
        },
        "availableTemplates": [{ "id": "noise" }]
    }))
    .unwrap();

    assert_eq!(
        workflow["activeTemplate"],
        serde_json::json!({"id": "bugfix"})
    );
    assert_eq!(workflow["currentStep"]["key"], "sweep");
    assert_eq!(workflow["currentStep"].as_object().unwrap().len(), 5);
    assert!(workflow.get("availableTemplates").is_none());
}

#[test]
fn slim_progress_reports_streaming_without_execution_as_active() {
    let mut state = state_with_execution(7, "advancing");
    state.execution = None;
    state.is_streaming = true;

    let progress = slim_progress(&state);

    assert_eq!(progress["state"], "active");
    assert_eq!(progress["reason"], "agent is running");
}

#[test]
fn since_equal_to_combined_streaming_generation_returns_unchanged_marker() {
    let mut state = state_with_execution(7, "advancing");
    state.generation = 9;

    let generation = slim_state_projection(&state)["generation"]
        .as_u64()
        .unwrap();
    let data = slim_state_response_data(&state, Some(generation));

    assert_eq!(
        data,
        serde_json::json!({"unchanged": true, "generation": generation})
    );
}
