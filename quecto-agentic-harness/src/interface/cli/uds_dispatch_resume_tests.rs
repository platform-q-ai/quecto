use super::*;
use crate::application::sessions::dto::{ResumeDecision, ResumeTarget};
use crate::domain::resume_decision::{HomeVersion, ResumeDecisionKind};
use crate::domain::session_home::SessionHomeScope;
use std::path::PathBuf;
fn json(event: &AgentEvent) -> serde_json::Value {
    serde_json::from_str(&event.to_json_line()).unwrap()
}
fn cross_folder(dir: &str) -> ResumeSavedSessionError {
    let target = ResumeTarget::parse("cli:foreign").unwrap();
    ResumeSavedSessionError::Decision(Box::new(ResumeDecision {
        home_version: HomeVersion::of(&target.identity, &SessionHomeScope::LegacyUnscoped),
        target,
        kind: ResumeDecisionKind::CrossFolder,
        execution_dir: Some(PathBuf::from(dir)),
        detail: Some("bell\u{7}".into()),
    }))
}
#[test]
fn a_cross_folder_obstacle_is_a_plain_typed_refusal_with_a_command() {
    let refusal = cross_folder("/work/else where");
    let event = json(&refusal_event(Some("r2045"), "resume_session", &refusal));
    assert_eq!(event["id"], "r2045");
    assert_eq!(event["success"], false);
    assert_eq!(
        event["data"],
        serde_json::json!({"outcome":"refused","code":"belongs_elsewhere","kind":"cross_folder",
            "session":"cli:foreign","sessionKey":"cli:foreign",
            "executionPath":"/work/else where","detail":"bell\u{fffd}",
            "command":"cd '/work/else where' && quecto-tui","resume":"/resume cli:foreign"})
    );
}
#[test]
fn a_folder_holding_a_terminal_control_is_shown_safely_and_has_no_command() {
    // #2056 review M4: the command is never sent unless it reads as it runs.
    let refusal = cross_folder("/work/else\u{1b}[2Jwhere");
    let data = json(&refusal_event(Some("r2045"), "resume_session", &refusal))["data"].clone();
    assert_eq!(data["executionPath"], "/work/else\u{fffd}[2Jwhere");
    assert_eq!(data["command"], serde_json::Value::Null);
    assert_eq!(data["resume"], "/resume cli:foreign");
    assert!(!data.to_string().contains('\u{1b}'), "{data}");
}
