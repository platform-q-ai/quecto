use super::*;
use tempfile::TempDir;

fn test_grep() -> (GrepTool, Arc<PathBuf>, TempDir) {
    let tmp = TempDir::new().unwrap();
    let ws = Arc::new(tmp.path().to_path_buf());
    let sandbox = Arc::new(Sandbox::new(Some(tmp.path().to_path_buf())));
    let tool = GrepTool::new(ws.clone(), sandbox);
    (tool, ws, tmp)
}

#[test]
fn test_format_grep_output_empty() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(format_grep_output(GrepFormatArgs {
        json_output: "",
        sandbox: &Sandbox::new(None),
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
        sandbox: &Sandbox::new(None),
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
        sandbox: &Sandbox::new(None),
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
async fn test_grep_outside_workspace_allowed() {
    let (tool, _ws, _tmp) = test_grep();
    let outside = tempfile::tempdir().unwrap();
    let file = outside.path().join("outside.txt");
    std::fs::write(&file, "needle\n").unwrap();
    let result = tool
        .execute(&format!(
            r#"{{"pattern": "needle", "path": "{}"}}"#,
            outside.path().display()
        ))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert!(result.content.contains("outside.txt"));
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

/// #2136: the tool is the one content search agents use; its description
/// says so, and its schema offers the rg options agents reach for in bash.
#[test]
fn the_definition_claims_all_content_search_and_offers_rg_options() {
    let (tool, _ws, _tmp) = test_grep();
    let definition = tool.definition();
    assert!(
        definition.description.contains(
            "USE THIS TOOL FOR ALL CONTENT SEARCH: do not run rg, grep or git grep through bash"
        ),
        "{}",
        definition.description
    );
    let schema: serde_json::Value = serde_json::from_str(&definition.parameters_schema).unwrap();
    let properties = schema["properties"].as_object().unwrap();
    for option in [
        "pattern",
        "patterns",
        "path",
        "glob",
        "type",
        "ignoreCase",
        "literal",
        "wordRegexp",
        "multiline",
        "maxPerFile",
        "output",
        "context",
        "limit",
    ] {
        assert!(properties.contains_key(option), "schema lacks {option}");
    }
    assert_eq!(
        schema["properties"]["output"]["enum"],
        serde_json::json!(["content", "files", "count"])
    );
}

/// #2136: a multiline match prints every line it spans as a match line.
#[test]
fn a_multiline_match_prints_every_line_it_spans() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let ws = tempfile::TempDir::new().unwrap();
    std::fs::write(ws.path().join("m.rs"), "a\nb(\n  c)\nd\n").unwrap();
    let json = format!(
        r#"{{"type":"match","data":{{"path":{{"text":"{}/m.rs"}},"line_number":2,"lines":{{"text":"b(\n  c)\n"}},"absolute_offset":2,"submatches":[]}}}}"#,
        ws.path().to_string_lossy()
    );
    let result = rt.block_on(format_grep_output(GrepFormatArgs {
        json_output: &json,
        sandbox: &Sandbox::new(None),
        workspace: ws.path(),
        match_limit: 100,
        context_lines: 1,
        max_line_bytes: 500,
        max_output_bytes: 50 * 1024,
    }));
    assert_eq!(result, "m.rs-1- a\nm.rs:2: b(\nm.rs:3:   c)\nm.rs-4- d");
}

/// A stand-in rg that prints `stdout` then exits with `code`.
fn fake_rg(dir: &std::path::Path, stdout_command: &str, code: i32) -> String {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("fake-rg.sh");
    std::fs::write(&path, format!("#!/bin/sh\n{stdout_command}\necho 'rg: ./locked: Permission denied (os error 13)' >&2\nexit {code}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path.to_string_lossy().into_owned()
}

/// #2136: rg's exit 2 after partial results (one unreadable file) returns
/// what it found with a notice, not an error; with nothing found it is one.
#[tokio::test]
async fn partial_results_before_an_rg_error_are_returned_with_a_notice() {
    let tmp = TempDir::new().unwrap();
    let ws = Arc::new(tmp.path().to_path_buf());
    std::fs::write(tmp.path().join("a.rs"), "needle\n").unwrap();
    let listing = format!("printf '%s\\0' '{}/a.rs'", tmp.path().display());
    let tool = GrepTool::with_rg_binary(
        ws.clone(),
        Arc::new(Sandbox::new(None)),
        fake_rg(tmp.path(), &listing, 2),
    );
    let result = tool
        .execute(r#"{"pattern": "needle", "output": "files"}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert!(result.content.starts_with("a.rs\n"), "{}", result.content);
    assert!(
        result.content.contains(
            "rg reported errors, results may be incomplete: rg: ./locked: Permission denied"
        ),
        "{}",
        result.content
    );
    let tool = GrepTool::with_rg_binary(
        ws,
        Arc::new(Sandbox::new(None)),
        fake_rg(tmp.path(), "true", 2),
    );
    let result = tool
        .execute(r#"{"pattern": "needle", "output": "files"}"#)
        .await
        .unwrap();
    assert!(result.is_error, "{}", result.content);
}

/// #2136: output past the read cap is reported, and a listing it cut off
/// keeps only complete records.
#[tokio::test]
async fn output_past_the_cap_is_reported_as_incomplete() {
    let tmp = TempDir::new().unwrap();
    let ws = Arc::new(tmp.path().to_path_buf());
    let flood = format!(
        "i=0; while [ $i -lt 20000 ]; do printf '%s/f%05d.rs\\0' '{}' $i; i=$((i+1)); done",
        tmp.path().display()
    );
    let tool = GrepTool::with_rg_binary(
        ws,
        Arc::new(Sandbox::new(None)),
        fake_rg(tmp.path(), &flood, 0),
    );
    let result = tool
        .execute(r#"{"pattern": "x", "output": "files", "limit": 5}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert!(
        result.content.contains("results are incomplete"),
        "{}",
        result.content
    );
    assert!(
        result.content.starts_with("f00000.rs\n"),
        "{}",
        result.content
    );
}
