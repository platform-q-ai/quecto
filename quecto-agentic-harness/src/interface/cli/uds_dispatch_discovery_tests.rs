use super::*;
use crate::application::sessions::dto::{ListSessionsResult, ListedSession};
use crate::domain::session::SessionSummary;
use crate::domain::session_home::SessionHomeScope;
use crate::interface::cli::protocol::SessionListScopeCommand;

#[test]
fn discovery_preserves_identity_and_authoritative_admission() {
    let result = ListSessionsResult {
        sessions: vec![ListedSession {
            summary: SessionSummary {
                key: "opaque:key".into(),
                identity: crate::domain::session_identity::SessionIdentity::from_persisted_key(
                    "opaque:key",
                ),
                title: "unsafe\u{1b}title".into(),
                message_count: 2,
                updated_unix_secs: Some(7),
            },
            home: SessionHomeScope::Unavailable("invalid".into()),
            resume_eligible: false,
        }],
        diagnostics: vec!["repair\nrequired".into()],
        rebuilt: true,
    };
    let value = discovery_json(&result, SessionListScopeCommand::Global);
    assert_eq!(value["scope"], "global");
    assert_eq!(value["sessions"][0]["key"], "opaque:key");
    assert_eq!(value["sessions"][0]["homeState"], "unavailable");
    assert_eq!(
        value["sessions"][0]["executionPath"],
        serde_json::Value::Null
    );
    assert_eq!(value["sessions"][0]["resumeEligible"], false);
    assert_eq!(value["sessions"][0]["title"], "unsafe�title");
    assert_eq!(value["diagnostics"][0], "repair�required");
    assert_eq!(value["rebuilt"], true);
    assert_eq!(safe_display(&"x".repeat(5000)).len(), 4096);
}
