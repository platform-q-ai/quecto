use super::*;
use crate::application::sessions::dto::{ResumeDecision, ResumeTarget};
use crate::domain::resume_decision::{HomeVersion, ResumeDecisionKind};
use crate::domain::session_home::SessionHomeScope;
use std::path::PathBuf;
fn json(event: &AgentEvent) -> serde_json::Value {
    serde_json::from_str(&event.to_json_line()).unwrap()
}
#[test]
fn a_cross_folder_obstacle_is_a_plain_typed_refusal_with_a_command() {
    let target = ResumeTarget::parse("cli:foreign").unwrap();
    let refusal = ResumeSavedSessionError::Decision(Box::new(ResumeDecision {
        home_version: HomeVersion::of(&target.identity, &SessionHomeScope::LegacyUnscoped),
        target,
        kind: ResumeDecisionKind::CrossFolder,
        execution_dir: Some(PathBuf::from("/work/else\u{1b}[2Jwhere")),
        detail: Some("bell\u{7}".into()),
    }));
    let event = json(&refusal_event(Some("r2045"), "resume_session", &refusal));
    assert_eq!(event["id"], "r2045");
    assert_eq!(event["success"], false);
    assert_eq!(
        event["data"],
        serde_json::json!({"outcome":"refused","code":"belongs_elsewhere","kind":"cross_folder","executionPath":"/work/else\u{fffd}[2Jwhere","detail":"bell\u{fffd}","command":"cd '/work/else\u{1b}[2Jwhere' && quecto-tui -s 'cli:foreign'"})
    );
}
