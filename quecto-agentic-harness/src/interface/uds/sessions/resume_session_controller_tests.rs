use super::*;
#[test]
fn exact_restore_preserves_target_and_version() {
    let raw = "h1-0123456789abcdef";
    let request = ResumeFields {
        session: "cli:saved".into(),
        expected_home_version: Some(raw.into()),
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
    }
    .into_request()
    .unwrap_err();
    assert_eq!(error.code(), "stale_home_version");
}
