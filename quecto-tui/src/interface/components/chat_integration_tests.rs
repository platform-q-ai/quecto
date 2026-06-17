use super::*;

fn strip_ansi(s: &str) -> String {
    let mut result = String::new();
    let mut in_escape = false;
    for ch in s.chars() {
        if in_escape {
            if ch.is_ascii_alphabetic() || ch == '~' {
                in_escape = false;
            }
        } else if ch == '\x1b' {
            in_escape = true;
        } else {
            result.push(ch);
        }
    }
    result
}

fn render_plain(chat: &mut Chat, width: usize) -> String {
    let lines = chat.render(width);
    lines
        .iter()
        .map(|l| strip_ansi(l))
        .collect::<Vec<_>>()
        .join("\n")
}

fn extract_result_text(result: &serde_json::Value) -> String {
    crate::infrastructure::client::extract_result_text(result)
}

/// Check that bg-colored lines (lines containing the bg ANSI code)
/// include the expected text.
fn bg_lines_contain(lines: &[String], bg_code: &str, expected: &str) -> bool {
    lines
        .iter()
        .filter(|l| l.contains(bg_code))
        .any(|l| strip_ansi(l).contains(expected))
}

#[test]
fn integration_read_tool_shows_content_in_box() {
    // Server sends this JSON for a read tool result.
    let end_json = serde_json::json!({
        "content": [{"type": "text", "text": "fn main() {\n    println!(\"hello\");\n}"}]
    });
    let result_text = extract_result_text(&end_json);

    let mut chat = Chat::new();
    chat.start_tool(
        "c-1".into(),
        "read".into(),
        r#"{"path":"src/main.rs"}"#.into(),
    );
    chat.complete_tool("c-1", &result_text, false, None);

    let lines = chat.render(80);
    let plain = render_plain(&mut chat, 80);

    // The read header should be in the box.
    assert!(
        bg_lines_contain(&lines, theme::BG_SUCCESS, "read"),
        "bg box should contain 'read' header: {}",
        plain
    );
    assert!(
        bg_lines_contain(&lines, theme::BG_SUCCESS, "src/main.rs"),
        "bg box should contain path: {}",
        plain
    );
    // The file content should also be in the bg box.
    assert!(
        bg_lines_contain(&lines, theme::BG_SUCCESS, "fn main()"),
        "bg box should contain file content 'fn main()': {}",
        plain
    );
}

#[test]
fn integration_read_tool_content_has_background() {
    let end_json = serde_json::json!({
        "content": [{"type": "text", "text": "line 1\nline 2\nline 3"}]
    });
    let result_text = extract_result_text(&end_json);

    let mut chat = Chat::new();
    chat.start_tool("c-1".into(), "read".into(), r#"{"path":"test.txt"}"#.into());
    chat.complete_tool("c-1", &result_text, false, None);

    let lines = chat.render(80);

    // Count how many lines have the success bg — should be more than just
    // the header (header + 3 content lines = at least 4).
    let bg_count = lines
        .iter()
        .filter(|l| l.contains(theme::BG_SUCCESS))
        .count();
    assert!(
        bg_count >= 4,
        "should have at least 4 bg lines (header + 3 content), got {}",
        bg_count
    );
}

#[test]
fn integration_edit_tool_shows_diff_in_box() {
    let end_json = serde_json::json!({
        "content": [{"type": "text", "text": "Applied edit\n+added line\n-removed line\n context"}]
    });
    let result_text = extract_result_text(&end_json);

    let mut chat = Chat::new();
    chat.start_tool(
        "c-1".into(),
        "edit".into(),
        r#"{"path":"src/main.rs"}"#.into(),
    );
    chat.complete_tool("c-1", &result_text, false, None);

    let lines = chat.render(80);
    let plain = render_plain(&mut chat, 80);

    assert!(
        bg_lines_contain(&lines, theme::BG_SUCCESS, "edit"),
        "bg box should contain 'edit' header: {}",
        plain
    );
    // Diff content should be inside the box.
    assert!(
        bg_lines_contain(&lines, theme::BG_SUCCESS, "+added"),
        "bg box should contain diff '+added': {}",
        plain
    );
    assert!(
        bg_lines_contain(&lines, theme::BG_SUCCESS, "-removed"),
        "bg box should contain diff '-removed': {}",
        plain
    );
}

#[test]
fn integration_bash_tool_shows_output_in_box() {
    let end_json = serde_json::json!({
        "content": [{"type": "text", "text": "file1.txt\nfile2.txt\nfile3.txt"}]
    });
    let result_text = extract_result_text(&end_json);

    let mut chat = Chat::new();
    chat.start_tool("c-1".into(), "bash".into(), r#"{"command":"ls"}"#.into());
    chat.complete_tool("c-1", &result_text, false, None);

    let lines = chat.render(80);
    let plain = render_plain(&mut chat, 80);

    assert!(
        bg_lines_contain(&lines, theme::BG_SUCCESS, "$ ls"),
        "bg box should contain '$ ls' header: {}",
        plain
    );
    assert!(
        bg_lines_contain(&lines, theme::BG_SUCCESS, "file1.txt"),
        "bg box should contain bash output: {}",
        plain
    );
}

#[test]
fn integration_write_tool_shows_content_in_box() {
    let end_json = serde_json::json!({
        "content": [{"type": "text", "text": "Written successfully"}]
    });
    let result_text = extract_result_text(&end_json);

    let mut chat = Chat::new();
    chat.start_tool(
        "c-1".into(),
        "write".into(),
        r#"{"path":"out.txt","content":"hello world\nsecond line"}"#.into(),
    );
    chat.complete_tool("c-1", &result_text, false, None);

    let lines = chat.render(80);
    let plain = render_plain(&mut chat, 80);

    assert!(
        bg_lines_contain(&lines, theme::BG_SUCCESS, "write"),
        "bg box should contain 'write' header: {}",
        plain
    );
    assert!(
        bg_lines_contain(&lines, theme::BG_SUCCESS, "out.txt"),
        "bg box should contain path: {}",
        plain
    );
    // Write shows the args content, not the result.
    assert!(
        bg_lines_contain(&lines, theme::BG_SUCCESS, "hello world"),
        "bg box should contain written content: {}",
        plain
    );
}

#[test]
fn integration_pending_tool_has_pending_bg() {
    // Before tool completes, should show pending bg.
    let mut chat = Chat::new();
    chat.start_tool("c-1".into(), "read".into(), r#"{"path":"test.txt"}"#.into());

    let lines = chat.render(80);
    let bg_count = lines
        .iter()
        .filter(|l| l.contains(theme::BG_PENDING))
        .count();
    assert!(
        bg_count >= 1,
        "pending tool should have pending bg, got {} bg lines",
        bg_count
    );
}

#[test]
fn integration_error_tool_has_error_bg() {
    let end_json = serde_json::json!({
        "content": [{"type": "text", "text": "command not found: xyz"}]
    });
    let result_text = extract_result_text(&end_json);

    let mut chat = Chat::new();
    chat.start_tool("c-1".into(), "bash".into(), r#"{"command":"xyz"}"#.into());
    chat.complete_tool("c-1", &result_text, true, None);

    let lines = chat.render(80);
    let bg_count = lines.iter().filter(|l| l.contains(theme::BG_ERROR)).count();
    assert!(
        bg_count >= 1,
        "error tool should have error bg, got {} bg lines",
        bg_count
    );
    assert!(
        bg_lines_contain(&lines, theme::BG_ERROR, "command not found"),
        "error bg box should contain error text"
    );
}

// ── Workflow tool rendering ────────────────────────────────────

#[test]
fn workflow_tool_renders_action_and_step() {
    let mut chat = Chat::new();
    chat.start_tool(
        "wf-1".into(),
        "workflow".into(),
        r#"{"action":"check","step":3}"#.into(),
    );
    chat.complete_tool("wf-1", "Step 3 checked.", false, Some(5));
    let lines = chat.render(80);
    assert!(
        bg_lines_contain(&lines, theme::BG_SUCCESS, "workflow"),
        "should contain 'workflow' in success bg"
    );
    assert!(
        bg_lines_contain(&lines, theme::BG_SUCCESS, "check"),
        "should contain action 'check'"
    );
}

#[test]
fn workflow_tool_renders_select_template() {
    let mut chat = Chat::new();
    chat.start_tool(
        "wf-2".into(),
        "workflow".into(),
        r#"{"action":"select_template","template":"feature"}"#.into(),
    );
    chat.complete_tool(
        "wf-2",
        "Selected workflow template 'feature'.",
        false,
        Some(10),
    );
    let lines = chat.render(80);
    assert!(
        bg_lines_contain(&lines, theme::BG_SUCCESS, "feature"),
        "should contain template name 'feature'"
    );
}

#[test]
fn workflow_tool_renders_set_issue() {
    let mut chat = Chat::new();
    chat.start_tool(
        "wf-3".into(),
        "workflow".into(),
        r#"{"action":"set_issue","issueNumber":42,"issueTitle":"Auth bug"}"#.into(),
    );
    chat.complete_tool("wf-3", "Active issue set: #42 — Auth bug", false, Some(3));
    let lines = chat.render(80);
    assert!(
        bg_lines_contain(&lines, theme::BG_SUCCESS, "#42"),
        "should contain issue number"
    );
}
