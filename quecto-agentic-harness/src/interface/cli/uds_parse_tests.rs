#[test]
fn parse_line_rejects_empty_and_oversized_lines() {
    match super::parse_line("   ") {
        super::LineResult::ParseError(err) => assert!(err.is_empty()),
        _ => panic!("expected empty parse error"),
    }

    let too_long = "x".repeat(super::MAX_LINE_BYTES + 1);
    assert!(matches!(
        super::parse_line(&too_long),
        super::LineResult::LineTooLong
    ));
}

#[test]
fn parse_line_parses_valid_command_and_reports_invalid_json() {
    match super::parse_line(r#"{"type":"abort","id":"a-1"}"#) {
        super::LineResult::Command(cmd) => {
            assert!(matches!(cmd, super::AgentCommand::Abort { .. }))
        }
        _ => panic!("expected abort command"),
    }

    match super::parse_line("not json") {
        super::LineResult::ParseError(err) => assert!(!err.is_empty()),
        _ => panic!("expected non-empty parse error"),
    }
}

#[test]
fn cancel_command_detection_matches_abort_and_steer_payloads() {
    assert!(super::is_cancel_command(r#"{"type":"abort"}"#));
    assert!(super::is_cancel_command(r#"{"type":"steer"}"#));
    assert!(!super::is_cancel_command(r#"{"type":"prompt"}"#));
}
