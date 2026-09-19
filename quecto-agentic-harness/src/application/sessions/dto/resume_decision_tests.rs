use super::*;

#[test]
fn restore_request_is_exact_and_has_no_version_by_default() {
    let request = ResumeRequest::restore("cli:saved");
    assert_eq!(request.target, "cli:saved");
    assert_eq!(request.expected_home_version, None);
}
