use super::*;
use crate::application::sessions::dto::{ResumeDecision, ResumeTarget};
use crate::domain::resume_decision::{HomeVersion, ResumeDecisionKind};
use crate::domain::session_home::SessionHomeScope;
use std::path::PathBuf;
fn json(event: &AgentEvent) -> serde_json::Value {
    serde_json::from_str(&event.to_json_line()).unwrap()
}
fn refused(kind: ResumeDecisionKind, dir: &str) -> ResumeSavedSessionError {
    let ResumeSavedSessionError::Decision(mut decision) = cross_folder(dir) else {
        unreachable!()
    };
    decision.kind = kind;
    ResumeSavedSessionError::Decision(decision)
}
/// #2056 review: going to the recorded folder resumes ONE kind. A changed home
/// may be this very folder and a missing one cannot be entered: they name the
/// folder for the reader and carry neither a command nor a resume step.
#[test]
fn only_a_session_that_lives_elsewhere_carries_a_command_and_a_resume_step() {
    for (kind, code) in [
        (ResumeDecisionKind::HomeMissing, "home_missing"),
        (ResumeDecisionKind::HomeChanged, "home_changed"),
        (ResumeDecisionKind::HomeUnknown, "home_unknown"),
        (ResumeDecisionKind::LegacyUnscoped, "no_home_recorded"),
    ] {
        let event = json(&refusal_event(
            Some("r"),
            "resume_session",
            &refused(kind, "/work/plain"),
        ));
        let data = &event["data"];
        assert_eq!(data["code"], code);
        assert_eq!(
            data["executionPath"], "/work/plain",
            "{code} still names the folder"
        );
        assert!(data["command"].is_null(), "{code}: {data}");
        assert!(data["resume"].is_null(), "{code}: {data}");
    }
    let event = json(&refusal_event(
        Some("r"),
        "resume_session",
        &refused(ResumeDecisionKind::CrossFolder, "/work/plain"),
    ));
    assert_eq!(event["data"]["command"], "cd '/work/plain' && quecto-tui");
    assert_eq!(event["data"]["resume"], "/resume cli:foreign");
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
