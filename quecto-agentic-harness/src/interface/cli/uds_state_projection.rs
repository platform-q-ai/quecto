use super::protocol::SessionState;

pub(crate) fn slim_workflow(value: &serde_json::Value) -> Option<serde_json::Value> {
    let active_template = value
        .get("activeTemplate")
        .or_else(|| value.get("active_template"))?;
    let template_id = active_template
        .get("id")
        .and_then(|v| v.as_str())
        .map(|id| serde_json::json!({"id": id}))
        .unwrap_or_else(|| active_template.clone());
    let mut workflow = serde_json::json!({"activeTemplate": template_id});
    if let Some(step) = value
        .get("currentStep")
        .or_else(|| value.get("current_step"))
    {
        let mut current_step = serde_json::Map::new();
        for key in ["index", "key", "label", "phase", "done"] {
            if let Some(v) = step.get(key) {
                current_step.insert(key.to_string(), v.clone());
            }
        }
        workflow["currentStep"] = serde_json::Value::Object(current_step);
    }
    Some(workflow)
}

pub(crate) fn slim_progress(state: &SessionState) -> serde_json::Value {
    if let Some(execution) = &state.execution {
        return serde_json::json!({
            "state": execution.progress.state,
            "reason": execution.progress.reason,
        });
    }
    serde_json::json!({
        "state": if state.is_streaming { "active" } else { "quiet" },
        "reason": if state.is_streaming {
            "agent is running"
        } else {
            "no tool activity in the last 120 seconds"
        },
    })
}

pub(crate) fn slim_state_projection(state: &SessionState) -> serde_json::Value {
    use crate::domain::state_snapshot::{SnapshotProgress, StateSnapshot};
    let progress = slim_progress(state);
    let snapshot = StateSnapshot {
        state: state
            .execution
            .as_ref()
            .map(|e| e.phase.clone())
            .unwrap_or_else(|| {
                if state.is_streaming {
                    "thinking"
                } else {
                    "idle"
                }
                .into()
            }),
        effort: state.effort.clone(),
        effort_levels: state.effort_levels.clone(),
        model: state.model.clone(),
        // The durable session key must survive slimming for TUI resume (#1534).
        session_key: state.session_key.clone(),
        progress: SnapshotProgress {
            state: progress["state"]
                .as_str()
                .expect("typed progress state")
                .into(),
            reason: progress["reason"]
                .as_str()
                .expect("typed progress reason")
                .into(),
        },
        generation: state.generation,
        workflow: state
            .workflow
            .as_ref()
            .and_then(slim_workflow)
            .and_then(|value| serde_json::from_value(value).ok()),
        control_receipts: state.control_receipts.clone(),
        automatic_turns_suspended: state.automatic_turns_suspended,
        repeated_failure_notifications: state.repeated_failure_notifications,
        admission: state.execution.as_ref().and_then(|e| e.admission.clone()),
    };
    serde_json::to_value(snapshot).expect("typed inspection snapshot serializes")
}

pub(crate) fn slim_state_response_data(
    state: &SessionState,
    since: Option<u64>,
) -> serde_json::Value {
    let data = slim_state_projection(state);
    let generation = data["generation"].as_u64().unwrap_or(state.generation);
    if since == Some(generation) {
        serde_json::json!({"unchanged": true, "generation": generation})
    } else {
        data
    }
}
