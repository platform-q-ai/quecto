//! Presenter of `resume_session` restored acknowledgements and typed refusals.
use super::super::uds_dispatch_query::safe_display;
use super::AgentEvent;
use crate::application::sessions::dto::{ResumeDecision, ResumeOutcome, ResumeSavedSessionError};
use crate::domain::sessions::value_objects::{session_open_command::*, session_path_text::*};

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
    // `command` opens quecto there resuming the session (`quecto-tui
    // --resume`, #2615); `resume` is the same typed inside an open quecto —
    // for the one kind going there resumes, and only when they read the way
    // they run.
    let there = decision.kind.resumes_by_opening_quecto_there();
    let folder = decision.execution_dir.as_deref().filter(|_| there);
    let command = folder.and_then(|folder| open_there_command(folder, &decision.target.name));
    let resume = folder.and_then(|_| resume_step(&decision.target.name));
    serde_json::json!({
        "outcome": "refused",
        "code": code,
        "session": safe_display(&decision.target.name),
        "sessionKey": decision.target.identity.runtime_key(),
        "kind": decision.kind.name(),
        "executionPath": path.as_deref().map(safe_display),
        "detail": decision.detail.as_deref().map(safe_display),
        "command": command,
        "resume": resume,
    })
}

#[cfg(test)]
#[path = "uds_dispatch_resume_tests.rs"]
mod tests;
