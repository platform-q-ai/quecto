// BDD step definitions for ReadTool Quecto-compatible scenarios (issue #144).
//
// Existing image-file steps (PNG/JPEG/GIF/WebP with the canonical extension)
// are in agent_tools_steps.rs. This module adds steps for:
//   - Byte-count and line-count file creation

use cucumber::given;

use super::*;

/// Create a file with approximately N bytes of multi-line ASCII content.
/// Each line is 40 bytes + newline = 41 bytes. N lines → ~41*N bytes.
/// Used to trigger the byte-truncation path (> 50 KB).
#[given(regex = r#"^a file "([^"]+)" exists with (\d+) bytes of content$"#)]
fn given_file_with_n_bytes(world: &mut QuectoWorld, filename: String, n: usize) {
    let ws = world
        .tool_workspace
        .as_ref()
        .expect("tool workspace not set");
    // Build multi-line content so no single line exceeds 50KB.
    // Each line: 40 'a' chars + '\n' = 41 bytes.
    let line = "a".repeat(40) + "\n";
    let num_lines = (n / line.len()).max(1);
    let content: String = line.repeat(num_lines);
    std::fs::write(ws.join(&filename), content).expect("write byte file");
}

/// Create a text file with exactly N numbered lines.
/// Used to test the user-limit truncation notice ("N more lines in file").
#[given(regex = r#"^a file "([^"]+)" exists with (\d+) lines$"#)]
fn given_file_with_n_lines(world: &mut QuectoWorld, filename: String, n: usize) {
    let ws = world
        .tool_workspace
        .as_ref()
        .expect("tool workspace not set");
    let content: String = (1..=n).map(|i| format!("line{}\n", i)).collect();
    std::fs::write(ws.join(&filename), content).expect("write line file");
}
