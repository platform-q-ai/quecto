use super::*;

fn tool_lines_have_status_glyph(lines: &[&String], glyph: &str) -> bool {
    lines.iter().any(|line| line.contains(glyph))
}

// ── Tool status glyphs ────────────────────────────────────────────

#[test]
fn running_tool_has_pending_status_glyph() {
    let mut chat = Chat::new();
    chat.start_tool("c-1".into(), "bash".into(), r#"{"command":"ls"}"#.into());
    let lines = chat.render(80);
    let tool_lines: Vec<_> = lines.iter().filter(|l| !l.is_empty()).collect();
    assert!(!tool_lines.is_empty());
    assert!(
        tool_lines_have_status_glyph(&tool_lines, "⠋"),
        "should have pending status glyph: {:?}",
        tool_lines
    );
}

#[test]
fn success_tool_has_success_status_glyph() {
    let mut chat = Chat::new();
    chat.start_tool("c-1".into(), "bash".into(), r#"{"command":"ls"}"#.into());
    chat.complete_tool("c-1", "ok", false, None);
    let lines = chat.render(80);
    let tool_lines: Vec<_> = lines.iter().filter(|l| !l.is_empty()).collect();
    assert!(
        tool_lines_have_status_glyph(&tool_lines, "✓"),
        "should have success status glyph: {:?}",
        tool_lines
    );
}

#[test]
fn error_tool_has_error_status_glyph() {
    let mut chat = Chat::new();
    chat.start_tool("c-1".into(), "bash".into(), r#"{"command":"ls"}"#.into());
    chat.complete_tool("c-1", "command not found", true, None);
    let lines = chat.render(80);
    let tool_lines: Vec<_> = lines.iter().filter(|l| !l.is_empty()).collect();
    assert!(
        tool_lines_have_status_glyph(&tool_lines, "✗"),
        "should have error status glyph: {:?}",
        tool_lines
    );
}
