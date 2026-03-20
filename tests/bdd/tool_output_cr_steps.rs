use super::*;

// ─── Tool output carriage return stripping BDD steps (#529) ──────────────────
//
// The actual stripping is done in quecto-tui (ToolOutput::set_result).
// These BDD steps verify the concept at the protocol/data level by testing
// the same stripping logic directly.

/// Replicate the TUI's strip_carriage_returns logic for BDD verification.
fn strip_carriage_returns(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for line in s.split('\n') {
        if !result.is_empty() {
            result.push('\n');
        }
        let trimmed = line.strip_suffix('\r').unwrap_or(line);
        if let Some(last_cr) = trimmed.rfind('\r') {
            result.push_str(&trimmed[last_cr + 1..]);
        } else {
            result.push_str(trimmed);
        }
    }
    result
}

#[given(expr = "a tool execution result with content {string}")]
fn given_tool_result_content(world: &mut QuectoWorld, content: String) {
    // Unescape the content string (BDD string escapes \r and \n literally)
    let unescaped = content.replace("\\r", "\r").replace("\\n", "\n");
    world.stdout = unescaped;
}

#[when("the content is processed for display")]
fn when_content_processed(world: &mut QuectoWorld) {
    world.stderr = strip_carriage_returns(&world.stdout);
}

#[then(expr = "the processed content should be {string}")]
fn then_processed_content(world: &mut QuectoWorld, expected: String) {
    let unescaped = expected.replace("\\r", "\r").replace("\\n", "\n");
    assert_eq!(
        world.stderr, unescaped,
        "processed content mismatch:\n  got:    {:?}\n  expect: {:?}",
        world.stderr, unescaped
    );
}
