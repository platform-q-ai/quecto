// Tests for --no-session flag (Issue #191).
use super::*;

#[test]
fn test_agent_no_session_flag_parsed() {
    let mut stderr = String::new();
    let a: Vec<String> = vec!["--no-session".into(), "-m".into(), "Hi".into()];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert!(flags.no_session, "expected no_session to be true");
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");
}

#[test]
fn test_agent_no_session_and_s_are_mutually_exclusive() {
    let mut stderr = String::new();
    let a: Vec<String> = vec![
        "--no-session".into(),
        "-s".into(),
        "mysession".into(),
        "-m".into(),
        "Hi".into(),
    ];
    let result = parse_agent_flags(&a, &mut stderr);
    assert!(
        result.is_none(),
        "expected None when --no-session and -s are combined"
    );
    assert!(
        stderr.contains("mutually exclusive"),
        "expected 'mutually exclusive' in stderr: {stderr}",
    );
}

#[test]
fn test_agent_no_session_default_is_false() {
    let mut stderr = String::new();
    let a: Vec<String> = vec!["-m".into(), "Hi".into()];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert!(
        !flags.no_session,
        "expected no_session to be false by default"
    );
}

#[test]
fn test_agent_no_session_leaves_session_name_none() {
    // --no-session sets no_session=true and session_name stays None
    let mut stderr = String::new();
    let a: Vec<String> = vec!["--no-session".into(), "-m".into(), "Hi".into()];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert!(flags.no_session);
    assert!(flags.session_name.is_none());
}

#[test]
fn test_agent_s_dash_still_works_as_ephemeral_alias() {
    let mut stderr = String::new();
    let a: Vec<String> = vec!["-s".into(), "-".into(), "-m".into(), "Hi".into()];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert_eq!(flags.session_name.as_deref(), Some("-"));
    assert!(!flags.no_session);
}

#[test]
fn test_agent_no_session_combined_with_other_flags() {
    let mut stderr = String::new();
    let a: Vec<String> = vec![
        "--no-session".into(),
        "--model".into(),
        "gpt-4o".into(),
        "-m".into(),
        "Hello".into(),
    ];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert!(flags.no_session);
    assert_eq!(flags.model_override.as_deref(), Some("gpt-4o"));
    assert_eq!(flags.message.as_deref(), Some("Hello"));
    assert!(stderr.is_empty());
}
