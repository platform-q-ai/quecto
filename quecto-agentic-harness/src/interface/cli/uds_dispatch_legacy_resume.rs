use super::AgentEvent;

pub(super) fn legacy_resume_action_event(id: Option<String>, command: String) -> AgentEvent {
    AgentEvent::Response {
        id,
        command,
        success: false,
        data: Some(serde_json::json!({
            "outcome": "refused",
            "code": "legacy_action_unsupported"
        })),
        error: Some("resume actions are no longer supported".to_string()),
    }
}
