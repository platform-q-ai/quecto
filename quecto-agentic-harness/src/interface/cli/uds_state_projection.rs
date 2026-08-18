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
    let mut data = serde_json::json!({
        "state": state
            .execution
            .as_ref()
            .map(|e| e.phase.as_str())
            .unwrap_or(if state.is_streaming { "thinking" } else { "idle" }),
        "effort": state.effort,
        "model": state.model,
        "progress": slim_progress(state),
    });
    if let Some(workflow) = state.workflow.as_ref().and_then(slim_workflow) {
        data["workflow"] = workflow;
    }
    data["generation"] = serde_json::json!(VisibleStateGeneration::from_state(state).get());
    data
}

pub(crate) struct VisibleStateGeneration(u64);

impl VisibleStateGeneration {
    pub(crate) fn fold_workflow_revision(state: &mut SessionState, workflow_revision: u64) {
        state.generation = state.generation.saturating_add(workflow_revision);
    }

    fn from_state(state: &SessionState) -> Self {
        // The public get_state cursor is one visible-state generation. Session,
        // workflow, and execution internals may maintain separate revisions,
        // but the slim projection owns their aggregation so callers never see a
        // phase-dependent cursor that can drop when streaming ends.
        let execution_generation = state
            .execution
            .as_ref()
            .map(|execution| execution.activity_generation)
            .unwrap_or(0);
        Self(state.generation.saturating_add(execution_generation))
    }

    fn get(self) -> u64 {
        self.0
    }
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
