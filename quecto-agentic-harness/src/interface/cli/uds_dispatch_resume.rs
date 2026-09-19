//! Presenter of `resume_session` restored acknowledgements and typed refusals.
use super::super::uds_dispatch_query::safe_display;
use super::AgentEvent;
use crate::application::sessions::dto::{ResumeDecision, ResumeOutcome, ResumeSavedSessionError};
use crate::domain::session_path_text::display_path;

pub(super) fn outcome_event(
    id: Option<&str>,
    command: &str,
    outcome: &ResumeOutcome,
) -> AgentEvent {
    let ResumeOutcome::Resumed(resumed) = outcome;
    AgentEvent::ok(
        id,
        command,
        Some(serde_json::json!({
            "outcome": "resumed",
            "session": resumed.name,
            "sessionKey": resumed.identity.runtime_key(),
            "messageCount": resumed.message_count,
        })),
    )
}

pub(super) fn refusal_event(
    id: Option<&str>,
    command: &str,
    refusal: &ResumeSavedSessionError,
) -> AgentEvent {
    let data = match refusal {
        ResumeSavedSessionError::Decision(decision) => decision_json(decision, refusal.code()),
        _ => serde_json::json!({ "outcome": "refused", "code": refusal.code() }),
    };
    AgentEvent::Response {
        id: id.map(str::to_owned),
        command: command.to_owned(),
        success: false,
        data: Some(data),
        error: Some(safe_display(&refusal.to_string())),
    }
}

fn decision_json(decision: &ResumeDecision, code: &str) -> serde_json::Value {
    let path = decision.execution_dir.as_deref().map(display_path);
    let command = decision.execution_dir.as_deref().and_then(|dir| {
        let raw = dir.to_str()?;
        Some(format!(
            "cd {} && quecto-tui -s {}",
            shell_quote(raw),
            shell_quote(&decision.target.name)
        ))
    });
    serde_json::json!({
        "outcome": "refused",
        "code": code,
        "kind": decision.kind.name(),
        "executionPath": path.as_deref().map(safe_display),
        "detail": decision.detail.as_deref().map(safe_display),
        "command": command,
    })
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
#[path = "uds_dispatch_resume_tests.rs"]
mod tests;
