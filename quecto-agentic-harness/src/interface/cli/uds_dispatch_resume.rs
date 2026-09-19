//! Presenter of the `resume_session` answers (#2011): the restored and
//! cancelled acknowledgements, the typed decision and every typed refusal. It
//! names wire fields and makes untrusted metadata safe; it decides nothing.
use super::super::uds_dispatch_query::safe_display;
use super::AgentEvent;
use crate::application::sessions::dto::{
    ActionAvailability, ResumeDecision, ResumeOutcome, ResumeSavedSessionError,
};
use crate::domain::session_path_text::display_path;

/// The answer to a request that was not refused.
pub(super) fn outcome_event(
    id: Option<&str>,
    command: &str,
    outcome: &ResumeOutcome,
) -> AgentEvent {
    let data = match outcome {
        // The name as spelled, the key the loop now stands for, the live length.
        ResumeOutcome::Resumed(resumed) => serde_json::json!({
            "outcome": "resumed",
            "session": resumed.name,
            "sessionKey": resumed.identity.runtime_key(),
            "messageCount": resumed.message_count,
        }),
        ResumeOutcome::Cancelled { name } => serde_json::json!({
            "outcome": "cancelled",
            "session": name,
        }),
    };
    AgentEvent::ok(id, command, Some(data))
}

/// The answer to a refused request: the safe text for people, and the typed
/// decision or refusal for clients.
pub(super) fn refusal_event(
    id: Option<&str>,
    command: &str,
    refusal: &ResumeSavedSessionError,
) -> AgentEvent {
    let code = refusal.code();
    let data = match refusal {
        ResumeSavedSessionError::Decision(decision) => decision_json(decision, code),
        ResumeSavedSessionError::ActionUnavailable { action, reason } => serde_json::json!({
            "outcome": "refused",
            "code": code,
            "action": action.name(),
            "reason": safe_display(reason),
        }),
        ResumeSavedSessionError::ActionExecutedElsewhere(action)
        | ResumeSavedSessionError::ActionNotOffered(action)
        | ResumeSavedSessionError::HomeVersionRequired(action) => serde_json::json!({
            "outcome": "refused",
            "code": code,
            "action": action.name(),
        }),
        _ => serde_json::json!({ "outcome": "refused", "code": code }),
    };
    AgentEvent::Response {
        id: id.map(str::to_owned),
        command: command.to_owned(),
        success: false,
        data: Some(data),
        error: Some(safe_display(&refusal.to_string())),
    }
}

fn shell_quote(value: &str) -> String {
    // POSIX single quotes are an allowlist: every byte is literal except the
    // quote, which is closed, escaped, and reopened without invoking a shell.
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn decision_json(decision: &ResumeDecision, code: &str) -> serde_json::Value {
    let actions: Vec<_> = decision
        .offers
        .iter()
        .map(|offer| {
            let reason = match &offer.availability {
                ActionAvailability::Available => None,
                ActionAvailability::Unavailable(reason) => Some(safe_display(reason)),
            };
            serde_json::json!({
                "action": offer.action.name(),
                "available": reason.is_none(),
                "reason": reason,
            })
        })
        .collect();
    // Spelled as every discovery row spells it (R3-H2): one folder, one text.
    let path = decision.execution_dir.as_deref().map(display_path);
    let command = path.as_deref().map(|folder| {
        format!("cd {} && quecto-tui -s {}", shell_quote(folder), shell_quote(&decision.target.identity.runtime_key()))
    });
    serde_json::json!({
        "outcome": "refused",
        "code": code,
        "session": safe_display(&decision.target.name),
        "sessionKey": decision.target.identity.runtime_key(),
        "kind": decision.kind.name(),
        "homeVersion": decision.home_version.as_str(),
        "executionPath": path.as_deref().map(safe_display),
        "detail": decision.detail.as_deref().map(safe_display),
        "command": command,
        "actions": actions,
    })
}

#[cfg(test)]
#[path = "uds_dispatch_resume_tests.rs"]
mod tests;
