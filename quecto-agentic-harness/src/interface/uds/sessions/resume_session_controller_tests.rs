use super::*;
#[test]
fn exact_restore_preserves_target_and_version() {
    let raw = "h1-0123456789abcdef";
    let request = ResumeFields {
        session: "cli:saved".into(),
        expected_home_version: Some(raw.into()),
        legacy_action: false,
    }
    .into_request()
    .unwrap();
    assert_eq!(request.target, "cli:saved");
    assert_eq!(request.expected_home_version.unwrap().as_str(), raw);
}
#[test]
fn malformed_version_is_stale() {
    let error = ResumeFields {
        session: "saved".into(),
        expected_home_version: Some("bad".into()),
        legacy_action: false,
    }
    .into_request()
    .unwrap_err();
    assert_eq!(error.code(), "stale_home_version");
}
/// #2045: a request that still names an action is refused before anything
/// else is read — a version that would parse, or one that would not, makes
/// no difference, and nothing is ever restored from it.
#[test]
fn a_legacy_action_is_refused_whatever_else_the_request_says() {
    for version in [None, Some("h1-0123456789abcdef"), Some("bad")] {
        let error = ResumeFields {
            session: "cli:saved".into(),
            expected_home_version: version.map(str::to_string),
            legacy_action: true,
        }
        .into_request()
        .unwrap_err();
        assert_eq!(error.code(), "legacy_action_unsupported", "{version:?}");
        assert_eq!(error.to_string(), "resume actions are no longer supported");
    }
    assert!(!ResumeFields::from("cli:saved").legacy_action);
}
