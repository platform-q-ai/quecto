use super::*;
use crate::domain::session_home::SessionHomeScope;

#[test]
fn a_bare_key_is_an_exact_restore() {
    for fields in [
        ResumeFields::from("chat-1-a"),
        ResumeFields::from("chat-1-a".to_string()),
    ] {
        let request = fields.into_request().unwrap();
        assert_eq!(request, ResumeRequest::restore("chat-1-a"));
    }
}

#[test]
fn an_action_and_a_version_are_carried_typed() {
    let version = HomeVersion::of(&SessionHomeScope::LegacyUnscoped);
    let request = ResumeFields {
        session: "cli:one".into(),
        action: Some(ResumeAction::ForkCurrent),
        expected_home_version: Some(version.as_str().to_string()),
    }
    .into_request()
    .unwrap();
    assert_eq!(request.target, "cli:one");
    assert_eq!(request.intent, ResumeIntent::Act(ResumeAction::ForkCurrent));
    assert_eq!(request.expected_home_version, Some(version));
}

#[test]
fn a_version_token_never_issued_is_stale_not_ignored() {
    for hostile in ["", "latest", "h1-XYZ", "h1-0123456789abcde\u{1b}"] {
        let refused = ResumeFields {
            session: "cli:one".into(),
            action: None,
            expected_home_version: Some(hostile.into()),
        }
        .into_request()
        .expect_err("refused");
        assert!(
            matches!(refused, ResumeSavedSessionError::StaleHomeVersion),
            "{hostile:?}"
        );
    }
}
