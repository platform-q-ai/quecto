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

/// Run a tool backed by a freshly written fake rg. Executing a script just
/// written can fail with ETXTBSY while another test thread's fork briefly
/// holds its write handle: retry that, and only that.
pub(super) async fn execute_fake(
    tool: &GrepTool,
    args: &str,
) -> Result<ToolResult, crate::domain::error::DomainError> {
    for _ in 0..50 {
        match tool.execute(args).await {
            Err(e) if e.to_string().contains("Text file busy") => {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            other => return other,
        }
    }
    tool.execute(args).await
}

/// A stand-in rg that prints `stdout` then exits with `code`.
pub(super) fn fake_rg(dir: &std::path::Path, stdout_command: &str, code: i32) -> String {
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
    let result = execute_fake(&tool, r#"{"pattern": "needle", "output": "files"}"#)
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
    let result = execute_fake(&tool, r#"{"pattern": "needle", "output": "files"}"#)
        .await
        .unwrap();
    assert!(result.is_error, "{}", result.content);
}

/// #2136: output past the read cap is reported, and a listing it cut off
/// keeps only complete records. When the match limit bound first, its own
/// notice (with the total read as a floor) is the only one.
#[tokio::test]
async fn output_past_the_cap_is_reported_as_incomplete() {
    let tmp = TempDir::new().unwrap();
    let flood = format!(
        "i=0; while [ $i -lt 20000 ]; do printf '%s/f%05d.rs\\0' '{}' $i; i=$((i+1)); done",
        tmp.path().display()
    );
    let tool = |limit: usize| {
        let tool = GrepTool::with_rg_binary(
            Arc::new(tmp.path().to_path_buf()),
            Arc::new(Sandbox::new(None)),
            fake_rg(tmp.path(), &flood, 0),
        );
        async move {
            execute_fake(
                &tool,
                &format!(r#"{{"pattern": "x", "output": "files", "limit": {limit}}}"#),
            )
            .await
            .unwrap()
        }
    };
    let bound = tool(5).await;
    assert!(!bound.is_error, "{}", bound.content);
    assert!(
        bound.content.starts_with("f00000.rs\n"),
        "{}",
        bound.content
    );
    assert!(
        bound.content.contains("5 of at least "),
        "{}",
        bound.content
    );
    assert!(
        !bound.content.contains("results are incomplete"),
        "{}",
        bound.content
    );
    let unbound = tool(100_000).await;
    assert!(
        unbound.content.contains("results are incomplete"),
        "{}",
        unbound.content
    );
}

/// #2136: a mistyped path under --json prints only a summary before exit 2:
/// that is an error, not "No matches found".
#[tokio::test]
async fn an_rg_error_with_only_a_summary_is_an_error() {
    let tmp = TempDir::new().unwrap();
    let summary =
        r#"printf '%s\n' '{"type":"summary","data":{"elapsed_total":{"secs":0,"nanos":1}}}'"#;
    let tool = GrepTool::with_rg_binary(
        Arc::new(tmp.path().to_path_buf()),
        Arc::new(Sandbox::new(None)),
        fake_rg(tmp.path(), summary, 2),
    );
    let result = execute_fake(&tool, r#"{"pattern": "needle"}"#)
        .await
        .unwrap();
    assert!(result.is_error, "{}", result.content);
    assert!(
        result.content.contains("Permission denied"),
        "{}",
        result.content
    );
}

/// #2136 round 2: rg writing far more stderr than a pipe holds never blocks
/// the tool, and a run past the timeout is killed and reported.
#[tokio::test]
async fn a_flood_of_rg_errors_never_blocks_and_a_slow_rg_is_stopped() {
    let tmp = TempDir::new().unwrap();
    let ws = Arc::new(tmp.path().to_path_buf());
    std::fs::write(tmp.path().join("a.rs"), "needle\n").unwrap();
    let flood = format!(
        "printf '%s\\0' '{}/a.rs'; i=0; while [ $i -lt 4000 ]; do echo \"rg: ./locked$i: Permission denied (os error 13)\" >&2; i=$((i+1)); done",
        tmp.path().display()
    );
    let tool = GrepTool::with_rg_binary(
        ws.clone(),
        Arc::new(Sandbox::new(None)),
        fake_rg(tmp.path(), &flood, 2),
    )
    .with_rg_timeout(std::time::Duration::from_secs(20));
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        execute_fake(&tool, r#"{"pattern": "needle", "output": "files"}"#),
    )
    .await
    .expect("the tool must not block on rg's stderr")
    .unwrap();
    assert!(result.content.starts_with("a.rs"), "{}", result.content);
    assert!(
        result.content.contains("rg: ./locked0: Permission denied"),
        "{}",
        result.content
    );
    let tool = GrepTool::with_rg_binary(
        ws,
        Arc::new(Sandbox::new(None)),
        fake_rg(tmp.path(), "exec sleep 30", 0),
    )
    .with_rg_timeout(std::time::Duration::from_millis(300));
    let started = std::time::Instant::now();
    let error = execute_fake(&tool, r#"{"pattern": "needle"}"#)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("rg did not finish within"), "{error}");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
}

/// #2136 round 3: a single matching line larger than the read cap is said
/// to be so, not reported as an rg failure.
#[tokio::test]
async fn a_matching_line_larger_than_the_cap_is_explained() {
    let tmp = TempDir::new().unwrap();
    let huge = format!(
        "printf '%s' '{{\"type\":\"match\",\"data\":{{\"path\":{{\"text\":\"{}/a.js\"}},\"lines\":{{\"text\":\"'; head -c 5000000 /dev/zero | tr '\\0' a; printf '%s\\n' '\"}},\"line_number\":1}}}}'",
        tmp.path().display()
    );
    let tool = GrepTool::with_rg_binary(
        Arc::new(tmp.path().to_path_buf()),
        Arc::new(Sandbox::new(None)),
        fake_rg(tmp.path(), &huge, 0),
    );
    let result = execute_fake(&tool, r#"{"pattern": "a"}"#).await.unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert!(
        result.content.contains("A matching line is larger than"),
        "{}",
        result.content
    );
}

/// #2136 round 3: rg stopped by a signal (not the tool's cap) returns what
/// it found with a notice that it is incomplete.
#[tokio::test]
async fn rg_stopped_by_a_signal_is_reported_incomplete() {
    let tmp = TempDir::new().unwrap();
    let listing = format!(
        "printf '%s\\0' '{}/a.rs'; kill -TERM $$",
        tmp.path().display()
    );
    let tool = GrepTool::with_rg_binary(
        Arc::new(tmp.path().to_path_buf()),
        Arc::new(Sandbox::new(None)),
        fake_rg(tmp.path(), &listing, 0),
    );
    let result = execute_fake(&tool, r#"{"pattern": "x", "output": "files"}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert!(result.content.starts_with("a.rs"), "{}", result.content);
    assert!(
        result.content.contains("rg was stopped by a signal"),
        "{}",
        result.content
    );
}

/// PR #2137 review (P1): a descendant that inherits rg's pipes cannot keep
/// the results waiting: past the cap rg is stopped and stderr, still held
/// open by the descendant, is abandoned after a short grace.
#[tokio::test]
async fn a_descendant_holding_the_pipes_cannot_hold_the_results() {
    let tmp = TempDir::new().unwrap();
    let flood = format!(
        "sleep 6 & i=0; while [ $i -lt 20000 ]; do printf '%s/f%05d.rs\\0' '{}' $i; i=$((i+1)); done; wait",
        tmp.path().display()
    );
    let tool = GrepTool::with_rg_binary(
        Arc::new(tmp.path().to_path_buf()),
        Arc::new(Sandbox::new(None)),
        fake_rg(tmp.path(), &flood, 0),
    )
    .with_rg_timeout(std::time::Duration::from_secs(30));
    let started = std::time::Instant::now();
    let result = execute_fake(&tool, r#"{"pattern": "x", "output": "files", "limit": 3}"#)
        .await
        .unwrap();
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    assert!(!result.is_error, "{}", result.content);
    assert!(
        result
            .content
            .starts_with("f00000.rs\nf00001.rs\nf00002.rs"),
        "{}",
        result.content
    );
}

/// PR #2137 review (P1): the tool's search is defined by its own flags; a
/// user's rg config (which could add --pre, a process per file) is ignored.
#[test]
fn rg_runs_without_a_users_config() {
    let request = grep_request::parse_request(&serde_json::json!({"pattern": "x"})).unwrap();
    let cmd = build_rg_command("rg", Path::new("/ws"), Path::new("/ws"), &request);
    let args: Vec<String> = cmd
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        args.first().map(String::as_str),
        Some("--no-config"),
        "{args:?}"
    );
    assert!(
        args.iter()
            .all(|a| a != "--pre" && a != "--search-zip" && a != "-z"),
        "{args:?}"
    );
}

/// PR #2137 review: stdout still held open after rg exits (a wrapper's
/// background child) is abandoned after a grace: the results found stand,
/// with a notice; a failure whose stderr was abandoned keeps its status.
#[tokio::test]
async fn a_pipe_held_open_after_rg_exits_is_abandoned_after_a_grace() {
    let tmp = TempDir::new().unwrap();
    let held = format!("sleep 6 & printf '%s\\0' '{}/a.rs'", tmp.path().display());
    let tool = GrepTool::with_rg_binary(
        Arc::new(tmp.path().to_path_buf()),
        Arc::new(Sandbox::new(None)),
        fake_rg(tmp.path(), &held, 0),
    )
    .with_rg_timeout(std::time::Duration::from_secs(30));
    let started = std::time::Instant::now();
    let result = execute_fake(&tool, r#"{"pattern": "x", "output": "files"}"#)
        .await
        .unwrap();
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    assert!(result.content.starts_with("a.rs"), "{}", result.content);
    assert!(
        result.content.contains("still held open after it exited"),
        "{}",
        result.content
    );

    let failed = GrepTool::with_rg_binary(
        Arc::new(tmp.path().to_path_buf()),
        Arc::new(Sandbox::new(None)),
        fake_rg(
            tmp.path(),
            "sleep 6 & exec 2>&-; exec 2>/dev/null; exit 2",
            2,
        ),
    )
    .with_rg_timeout(std::time::Duration::from_secs(30));
    let result = execute_fake(&failed, r#"{"pattern": "x", "output": "files"}"#)
        .await
        .unwrap();
    assert!(result.is_error, "{}", result.content);
    assert!(
        result
            .content
            .contains("rg failed with exit status 2 and no message"),
        "{}",
        result.content
    );
}
