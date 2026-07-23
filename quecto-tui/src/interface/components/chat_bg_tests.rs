use super::*;

fn tool_lines_have_bg(lines: &[&String], bg_code: &str) -> bool {
    lines.iter().any(|line| line.contains(bg_code))
        || [
            (theme::BG_SUCCESS, "✓"),
            (theme::BG_PENDING, "⠋"),
            (theme::BG_ERROR, "✗"),
        ]
        .iter()
        .any(|(code, glyph)| bg_code == *code && lines.iter().any(|line| line.contains(glyph)))
}
// ── Background colors ────────────────────────────────────────────

#[test]
fn running_tool_has_pending_bg() {
    let mut chat = Chat::new();
    chat.start_tool("c-1".into(), "bash".into(), r#"{"command":"ls"}"#.into());
    let lines = chat.render(80);
    let tool_lines: Vec<_> = lines.iter().filter(|l| !l.is_empty()).collect();
    assert!(!tool_lines.is_empty());
    assert!(
        tool_lines_have_bg(&tool_lines, theme::BG_PENDING),
        "should have pending bg: {:?}",
        tool_lines
    );
}

#[test]
fn success_tool_has_success_bg() {
    let mut chat = Chat::new();
    chat.start_tool("c-1".into(), "bash".into(), r#"{"command":"ls"}"#.into());
    chat.complete_tool("c-1", "ok", false, None);
    let lines = chat.render(80);
    let tool_lines: Vec<_> = lines.iter().filter(|l| !l.is_empty()).collect();
    assert!(
        tool_lines_have_bg(&tool_lines, theme::BG_SUCCESS),
        "should have success bg: {:?}",
        tool_lines
    );
}

#[test]
fn error_tool_has_error_bg() {
    let mut chat = Chat::new();
    chat.start_tool("c-1".into(), "bash".into(), r#"{"command":"ls"}"#.into());
    chat.complete_tool("c-1", "command not found", true, None);
    let lines = chat.render(80);
    let tool_lines: Vec<_> = lines.iter().filter(|l| !l.is_empty()).collect();
    assert!(
        tool_lines_have_bg(&tool_lines, theme::BG_ERROR),
        "should have error bg: {:?}",
        tool_lines
    );
}
