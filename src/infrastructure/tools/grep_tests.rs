use super::*;
use tempfile::TempDir;

fn test_grep() -> (GrepTool, Arc<PathBuf>, TempDir) {
    let tmp = TempDir::new().unwrap();
    let ws = Arc::new(tmp.path().to_path_buf());
    let sandbox = Arc::new(Sandbox::new(Some(tmp.path().to_path_buf()), true));
    let tool = GrepTool::new(ws.clone(), sandbox);
    (tool, ws, tmp)
}

#[test]
fn test_format_grep_output_empty() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(format_grep_output(GrepFormatArgs {
        json_output: "",
        sandbox: &Sandbox::new(None, false),
        workspace: &PathBuf::from("/ws"),
        match_limit: 100,
        context_lines: 0,
        max_line_bytes: 500,
        max_output_bytes: 50 * 1024,
    }));
    assert_eq!(result, "No matches found");
}

#[test]
fn test_format_grep_output_basic() {
    // Single match event
    let json = r#"{"type":"match","data":{"path":{"text":"/ws/main.rs"},"line_number":1,"lines":{"text":"fn main() {}\n"},"absolute_offset":0,"submatches":[]}}"#;
    let rt = tokio::runtime::Runtime::new().unwrap();
    let ws = tempfile::TempDir::new().unwrap();
    std::fs::write(ws.path().join("main.rs"), "fn main() {}\n").unwrap();
    let json_patched = json.replace("/ws", &ws.path().to_string_lossy());
    let result = rt.block_on(format_grep_output(GrepFormatArgs {
        json_output: &json_patched,
        sandbox: &Sandbox::new(None, false),
        workspace: ws.path(),
        match_limit: 100,
        context_lines: 0,
        max_line_bytes: 500,
        max_output_bytes: 50 * 1024,
    }));
    assert!(result.contains("main.rs:1:"), "got: {}", result);
}

#[test]
fn test_format_grep_output_match_limit() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let ws = tempfile::TempDir::new().unwrap();
    let mut json_lines = Vec::new();
    for i in 1..=20usize {
        std::fs::write(
            ws.path().join(format!("f{}.rs", i)),
            format!("needle {}\n", i),
        )
        .unwrap();
        json_lines.push(format!(
            r#"{{"type":"match","data":{{"path":{{"text":"{}/f{}.rs"}},"line_number":1,"lines":{{"text":"needle {}\n"}},"absolute_offset":0,"submatches":[]}}}}"#,
            ws.path().to_string_lossy(), i, i
        ));
    }
    let json = json_lines.join("\n");
    let result = rt.block_on(format_grep_output(GrepFormatArgs {
        json_output: &json,
        sandbox: &Sandbox::new(None, false),
        workspace: ws.path(),
        match_limit: 5,
        context_lines: 0,
        max_line_bytes: 500,
        max_output_bytes: 50 * 1024,
    }));
    assert!(
        result.contains("5 matches limit reached"),
        "expected match limit notice, got: {}",
        &result[..result.len().min(200)]
    );
    assert!(
        result.contains("limit=10"),
        "expected limit=10, got: {}",
        result
    );
}

#[test]
fn test_truncate_line_short() {
    let (result, was_truncated) = truncate_line("hello", 500);
    assert_eq!(result, "hello");
    assert!(!was_truncated);
}

#[test]
fn test_truncate_line_long() {
    let long = "x".repeat(600);
    let (result, was_truncated) = truncate_line(&long, 500);
    assert!(result.contains("…"), "expected ellipsis");
    assert!(result.len() < 600);
    assert!(was_truncated);
}

#[test]
fn test_parse_rg_matches() {
    let json = r#"{"type":"begin","data":{"path":{"text":"/ws/a.rs"}}}
{"type":"match","data":{"path":{"text":"/ws/a.rs"},"line_number":3,"lines":{"text":"target\n"},"absolute_offset":10,"submatches":[]}}
{"type":"end","data":{"path":{"text":"/ws/a.rs"},"stats":{}}}
{"type":"summary","data":{}}"#;
    let matches = parse_rg_matches(json);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].line_number, 3);
    assert_eq!(matches[0].file_path, PathBuf::from("/ws/a.rs"));
}

#[tokio::test]
async fn test_grep_match_limit_notice_format() {
    let (tool, _ws, tmp) = test_grep();
    let content: String = (1..=50).map(|i| format!("needle {}\n", i)).collect();
    std::fs::write(tmp.path().join("many.txt"), content).unwrap();

    if std::process::Command::new("rg")
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }

    let result = tool
        .execute(r#"{"pattern": "needle", "limit": 5}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "got error: {}", result.content);
    assert!(
        result.content.contains("5 matches limit reached"),
        "expected match limit notice, got: {}",
        result.content
    );
    assert!(
        result.content.contains("limit=10"),
        "expected suggested limit, got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_grep_context_uses_file_minus_format() {
    let (tool, _ws, tmp) = test_grep();
    std::fs::write(
        tmp.path().join("ctx.rs"),
        "line one\nfn target() {}\nline three\n",
    )
    .unwrap();

    if std::process::Command::new("rg")
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }

    let result = tool
        .execute(r#"{"pattern": "target", "context": 1}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "got: {}", result.content);
    assert!(
        result.content.contains("ctx.rs:2:"),
        "expected match line format, got: {}",
        result.content
    );
    assert!(
        result.content.contains("ctx.rs-1-") || result.content.contains("ctx.rs-3-"),
        "expected context line format (file-N-), got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_grep_finds_pattern() {
    let (tool, _ws, tmp) = test_grep();
    std::fs::write(
        tmp.path().join("hello.rs"),
        "fn hello() { println!(\"hi\"); }\n",
    )
    .unwrap();

    // Skip if rg not available
    if std::process::Command::new("rg")
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }

    let result = tool.execute(r#"{"pattern": "hello"}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("hello"), "got: {}", result.content);
}

#[tokio::test]
async fn test_grep_no_matches() {
    let (tool, _ws, tmp) = test_grep();
    std::fs::write(tmp.path().join("file.rs"), "fn nothing() {}\n").unwrap();

    if std::process::Command::new("rg")
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }

    let result = tool
        .execute(r#"{"pattern": "xyz_nonexistent_9999"}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(
        result.content.contains("No matches found"),
        "got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_grep_outside_workspace_blocked() {
    let (tool, _ws, _tmp) = test_grep();
    let result = tool.execute(r#"{"pattern": "root", "path": "/etc"}"#).await;
    assert!(result.is_err() || result.unwrap().is_error);
}

// --- Fix 2: Actionable missing-parameter error ---

#[tokio::test]
async fn test_grep_empty_object_returns_actionable_error() {
    let (tool, _ws, _tmp) = test_grep();
    let result = tool.execute("{}").await.unwrap();
    assert!(result.is_error, "expected error, got: {}", result.content);
    assert!(
        result.content.contains("pattern"),
        "should mention missing 'pattern', got: {}",
        result.content
    );
    assert!(
        result.content.contains("Example"),
        "should include example, got: {}",
        result.content
    );
}

// --- Fix 3: Description includes example ---

#[test]
fn test_grep_description_includes_example() {
    let (tool, _ws, _tmp) = test_grep();
    let def = tool.definition();
    assert!(
        def.description.contains("Example"),
        "grep description should include Example, got: {}",
        def.description
    );
}
