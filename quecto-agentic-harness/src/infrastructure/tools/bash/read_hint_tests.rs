//! #2196 review: the bash commands `read` suggests for what it cannot show
//! still bring back the bytes they name, through the real bash tool.
use crate::application::tools::ports::Tool;
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::bash::ExecTool;
use crate::infrastructure::tools::filesystem::ReadTool;
use std::path::PathBuf;
use std::sync::Arc;

const BUDGET: usize = 50 * 1024;

fn tools() -> (ReadTool, ExecTool, tempfile::TempDir) {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Arc::new(PathBuf::from(tmp.path()));
    let sandbox = Arc::new(Sandbox::new(Some(tmp.path().to_path_buf())));
    let read = ReadTool::new(workspace.clone(), sandbox.clone());
    let exec = ExecTool::new(workspace, sandbox);
    (read, exec, tmp)
}

/// The command after "Use bash: " in a `read` answer, without the closing `]`.
fn suggested_command(answer: &str) -> String {
    let command = answer
        .split_once("Use bash: ")
        .unwrap_or_else(|| panic!("no bash hint in {answer}"))
        .1;
    command.strip_suffix(']').unwrap_or(command).to_string()
}

async fn run(exec: &ExecTool, command: &str) -> String {
    let args = serde_json::json!({ "command": command }).to_string();
    let result = exec.execute(&args).await.unwrap();
    assert!(!result.is_error, "{}", result.content);
    result.content
}

/// A line over 50KB: `read` names it and suggests `sed -n 'Np' … | head -c`;
/// that command returns the line's first 50KB, whole.
#[tokio::test]
async fn the_over_long_line_hint_brings_back_the_line() {
    let (read, exec, tmp) = tools();
    let line: String = (0..70_000)
        .map(|i| char::from(b'a' + (i % 26) as u8))
        .collect();
    std::fs::write(tmp.path().join("wide.txt"), format!("short\n{line}\nend\n")).unwrap();
    let answer = read
        .execute(r#"{"path": "wide.txt", "offset": 2}"#)
        .await
        .unwrap()
        .content;
    let command = suggested_command(&answer);
    assert!(command.starts_with("sed -n '2p'"), "{command}");
    assert_eq!(run(&exec, &command).await, line[..BUDGET]);
}

/// A file over 10 MiB: `read` refuses it and suggests `head -n 2000 … |
/// head -c`; that command returns the file's first 50KB, long lines whole.
#[tokio::test]
async fn the_too_large_file_hint_brings_back_its_start() {
    let (read, exec, tmp) = tools();
    let line = "x".repeat(20_000);
    let body = format!("{line}\n").repeat(600);
    std::fs::write(tmp.path().join("big.log"), &body).unwrap();
    let answer = read
        .execute(r#"{"path": "big.log"}"#)
        .await
        .unwrap()
        .content;
    let command = suggested_command(&answer);
    assert!(command.starts_with("head -n 2000"), "{command}");
    assert_eq!(run(&exec, &command).await, body[..BUDGET]);
}
