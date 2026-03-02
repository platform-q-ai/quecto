use super::*;
use std::path::Path;
use tempfile::TempDir;

fn test_find() -> (FindTool, Arc<PathBuf>, TempDir) {
    let tmp = TempDir::new().unwrap();
    let ws = Arc::new(tmp.path().to_path_buf());
    let sandbox = Arc::new(Sandbox::new(Some(tmp.path().to_path_buf()), true));
    let tool = FindTool::new(ws.clone(), sandbox);
    (tool, ws, tmp)
}

fn fd_available() -> bool {
    std::process::Command::new("fd")
        .arg("--version")
        .output()
        .is_ok()
}

// --- format_find_output unit tests ---

fn fmt(raw: &str, dir: &str, limit: usize, cap: usize) -> String {
    format_find_output(raw, Path::new(dir), limit, cap)
}

#[test]
fn test_format_find_empty() {
    assert_eq!(
        fmt("", "/ws", 1000, 50 * 1024),
        "No files found matching pattern"
    );
}

#[test]
fn test_format_find_whitespace_only() {
    assert_eq!(
        fmt("   \n\n", "/ws", 1000, 50 * 1024),
        "No files found matching pattern"
    );
}

#[test]
fn test_format_find_relativises_path() {
    let raw = "/ws/src/main.rs\n/ws/lib.rs";
    let result = fmt(raw, "/ws", 1000, 50 * 1024);
    assert!(result.contains("src/main.rs"), "got: {}", result);
    assert!(result.contains("lib.rs"), "got: {}", result);
    assert!(
        !result.contains("/ws/"),
        "should not contain absolute ws prefix: {}",
        result
    );
}

#[test]
fn test_format_find_limit_hint() {
    // Simulate exactly `limit` lines returned — fd capped at limit.
    let lines: Vec<String> = (1..=10).map(|i| format!("/ws/file{}.rs", i)).collect();
    let raw = lines.join("\n");
    let result = fmt(&raw, "/ws", 10, 50 * 1024);
    assert!(
        result.contains("10 results limit reached"),
        "expected limit hint, got: {}",
        result
    );
}

#[test]
fn test_format_find_byte_cap() {
    let lines: Vec<String> = (1..=200).map(|i| format!("/ws/file{}.txt", i)).collect();
    let raw = lines.join("\n");
    let result = fmt(&raw, "/ws", 1000, 512);
    assert!(
        result.contains("limit reached"),
        "expected byte-cap hint, got: {}",
        result
    );
}

#[test]
fn test_format_find_directory_trailing_slash() {
    // fd outputs "subdir/" for directory entries — we preserve the slash.
    let raw = "/ws/subdir/";
    let result = fmt(raw, "/ws", 1000, 50 * 1024);
    assert!(result.contains("subdir/"), "got: {}", result);
}

// --- Pi parity: nested .gitignore and float limit ---

#[test]
fn test_discover_nested_gitignore_finds_files() {
    let tmp = TempDir::new().unwrap();
    // Create nested .gitignore files
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::write(tmp.path().join(".gitignore"), "target/\n").unwrap();
    std::fs::write(tmp.path().join("src/.gitignore"), "generated/\n").unwrap();
    let found = discover_gitignore_files(tmp.path());
    // Should find both .gitignore files
    assert!(!found.is_empty(), "should find at least root .gitignore");
    let has_src = found.iter().any(|p| p.ends_with("src/.gitignore"));
    assert!(has_src, "should find src/.gitignore, got: {:?}", found);
}

#[test]
fn test_discover_nested_gitignore_empty_dir() {
    let tmp = TempDir::new().unwrap();
    let found = discover_gitignore_files(tmp.path());
    assert!(found.is_empty(), "no .gitignore → empty list");
}

#[test]
fn test_discover_gitignore_excludes_catch_all() {
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("sub")).unwrap();
    // Legitimate gitignore
    std::fs::write(tmp.path().join(".gitignore"), "target/\n").unwrap();
    // Catch-all gitignore (blocks everything)
    std::fs::write(tmp.path().join("sub/.gitignore"), "*\n!.gitignore\n").unwrap();
    let found = discover_gitignore_files(tmp.path());
    // The catch-all should be excluded
    let catch_all = found.iter().any(|p| p.ends_with("sub/.gitignore"));
    assert!(
        !catch_all,
        "catch-all gitignore (*) should be excluded from --ignore-file list, got: {:?}",
        found
    );
    // The legitimate one should be included
    let has_root = found
        .iter()
        .any(|p| p.ends_with(".gitignore") && !p.ends_with("sub/.gitignore"));
    assert!(
        has_root,
        "legitimate gitignore should be included, got: {:?}",
        found
    );
}

#[test]
fn test_discover_gitignore_excludes_bare_star_only() {
    let tmp = TempDir::new().unwrap();
    // A gitignore that is just "*" with no negations
    std::fs::write(tmp.path().join(".gitignore"), "*\n").unwrap();
    let found = discover_gitignore_files(tmp.path());
    assert!(
        found.is_empty(),
        "bare '*' gitignore should be excluded, got: {:?}",
        found
    );
}

#[test]
fn test_discover_gitignore_excludes_double_star_variants() {
    let tmp = TempDir::new().unwrap();
    // ** is equivalent to * in gitignore scope
    std::fs::write(tmp.path().join(".gitignore"), "**\n!.gitignore\n").unwrap();
    let found = discover_gitignore_files(tmp.path());
    assert!(
        found.is_empty(),
        "'**' catch-all gitignore should be excluded, got: {:?}",
        found
    );
}

#[test]
fn test_is_catch_all_gitignore_double_star_slash_variants() {
    use crate::infrastructure::tools::find::is_catch_all_gitignore;
    let tmp = TempDir::new().unwrap();

    for pattern in &["**", "**/", "**/*"] {
        let p = tmp.path().join(".gitignore");
        std::fs::write(&p, format!("{}\n!.gitignore\n", pattern)).unwrap();
        assert!(
            is_catch_all_gitignore(&p),
            "pattern '{}' should be detected as catch-all",
            pattern
        );
    }
}

#[test]
fn test_is_catch_all_gitignore_oversized_file_is_not_excluded() {
    use crate::infrastructure::tools::find::is_catch_all_gitignore;
    let tmp = TempDir::new().unwrap();
    let p = tmp.path().join(".gitignore");
    // Write a file larger than 64 KiB — should not be treated as catch-all
    // (safe default: include it, let fd decide)
    let big = "target/\n".repeat(10_000); // ~80 KiB
    std::fs::write(&p, big).unwrap();
    assert!(
        !is_catch_all_gitignore(&p),
        "oversized gitignore should default to non-catch-all (include it)"
    );
}

#[tokio::test]
async fn test_find_not_suppressed_by_catchall_gitignore_in_subdir() {
    let (tool, _ws, tmp) = test_find();
    std::fs::write(tmp.path().join("notes.txt"), "").unwrap();
    std::fs::create_dir_all(tmp.path().join("sub")).unwrap();
    // Catch-all gitignore in a subdirectory
    std::fs::write(tmp.path().join("sub/.gitignore"), "*\n!.gitignore\n").unwrap();

    if !fd_available() {
        return;
    }

    let result = tool.execute(r#"{"pattern": "*.txt"}"#).await.unwrap();
    assert!(!result.is_error, "got: {}", result.content);
    assert!(
        result.content.contains("notes.txt"),
        "notes.txt should be found despite catch-all gitignore in sub/, got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_find_float_limit_accepted() {
    let (tool, _ws, tmp) = test_find();
    for i in 0..10 {
        std::fs::write(tmp.path().join(format!("file_{:04}.txt", i)), "").unwrap();
    }
    if !fd_available() {
        return;
    }
    // Pass float limit via JSON
    let result = tool
        .execute(r#"{"pattern": "*.txt", "limit": 5.0}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "got: {}", result.content);
    assert!(
        result.content.contains("limit"),
        "expected limit notice, got: {}",
        result.content
    );
}

// --- tool integration tests (require fd on PATH) ---

#[tokio::test]
async fn test_find_glob_matches() {
    let (tool, _ws, tmp) = test_find();
    std::fs::write(tmp.path().join("hello.rs"), "fn main() {}").unwrap();
    std::fs::write(tmp.path().join("notes.txt"), "notes").unwrap();

    if !fd_available() {
        return; // fd not installed — skip
    }

    let result = tool.execute(r#"{"pattern": "*.rs"}"#).await.unwrap();
    assert!(!result.is_error, "got: {}", result.content);
    assert!(
        result.content.contains("hello.rs"),
        "got: {}",
        result.content
    );
    assert!(
        !result.content.contains("notes.txt"),
        "got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_find_no_matches() {
    let (tool, _ws, tmp) = test_find();
    std::fs::write(tmp.path().join("only.txt"), "text").unwrap();

    if !fd_available() {
        return;
    }

    let result = tool
        .execute(r#"{"pattern": "*.xyz_nonexistent"}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "got: {}", result.content);
    assert!(
        result.content.contains("No files found"),
        "got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_find_outside_workspace_blocked() {
    let (tool, _ws, _tmp) = test_find();
    let result = tool
        .execute(r#"{"pattern": "*.conf", "path": "/etc"}"#)
        .await;
    assert!(result.is_err() || result.unwrap().is_error);
}

// --- Actionable missing-parameter error ---

#[tokio::test]
async fn test_find_empty_object_returns_actionable_error() {
    let (tool, _ws, _tmp) = test_find();
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

// --- Description includes example ---

#[test]
fn test_find_description_includes_example() {
    let (tool, _ws, _tmp) = test_find();
    let def = tool.definition();
    assert!(
        def.description.contains("Example"),
        "find description should include Example, got: {}",
        def.description
    );
}

// --- Bug 1: path-segment glob support ---

#[test]
fn test_pattern_contains_slash_detected() {
    // Patterns with a slash should trigger full-path mode.
    assert!(pattern_has_path_segment("nested/*.txt"));
    assert!(pattern_has_path_segment("src/main.rs"));
    assert!(pattern_has_path_segment("a/b/c/*.rs"));
    // Patterns without a slash should NOT trigger full-path mode.
    assert!(!pattern_has_path_segment("*.txt"));
    assert!(!pattern_has_path_segment("*.rs"));
    assert!(!pattern_has_path_segment("main.rs"));
}

#[test]
fn test_double_star_slash_triggers_full_path() {
    // **/ patterns also contain a slash and should use full-path + no extra prepend.
    assert!(pattern_has_path_segment("**/*.rs"));
    assert!(pattern_has_path_segment("**/nested/file.txt"));
}

#[test]
fn test_prepend_glob_star_for_non_anchored_patterns() {
    // Patterns with slash but not starting with ** or / should get **/ prepended.
    assert_eq!(build_full_path_pattern("src/*.rs"), "**/src/*.rs");
    assert_eq!(build_full_path_pattern("nested/*.txt"), "**/nested/*.txt");
    // Already anchored with ** — no change.
    assert_eq!(build_full_path_pattern("**/*.rs"), "**/*.rs");
    assert_eq!(build_full_path_pattern("**/src/*.rs"), "**/src/*.rs");
    // No slash — no change (full-path mode not used for these).
    assert_eq!(build_full_path_pattern("*.rs"), "*.rs");
}

#[test]
fn test_dot_slash_prefix_stripped() {
    // LLMs sometimes emit ./relative paths; strip ./ before anchoring.
    assert_eq!(build_full_path_pattern("./src/*.rs"), "**/src/*.rs");
    assert_eq!(build_full_path_pattern("./nested/*.txt"), "**/nested/*.txt");
    // ./ prefix on an already-anchored pattern — strip ./ then pass through.
    assert_eq!(build_full_path_pattern("./**/*.rs"), "**/*.rs");
}

#[test]
fn test_absolute_slash_prefix_stripped() {
    // Patterns starting with / are stripped so they can be anchored with **/.
    // An unstripped /src/*.rs would never match /workspace/src/foo.rs.
    assert_eq!(build_full_path_pattern("/src/*.rs"), "**/src/*.rs");
    assert_eq!(build_full_path_pattern("/nested/*.txt"), "**/nested/*.txt");
}

#[test]
fn test_traversal_pattern_gets_star_prepend() {
    // Even traversal-looking patterns are safe — fd treats .. as a literal glob
    // segment and never traverses outside the specified search directory.
    // We document the invariant: these patterns are normalised, not rejected.
    assert_eq!(
        build_full_path_pattern("../../etc/*.conf"),
        "**/../../etc/*.conf"
    );
}

#[tokio::test]
async fn test_find_path_segment_glob_matches() {
    let (tool, _ws, tmp) = test_find();
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::write(tmp.path().join("src/main.rs"), "fn main() {}").unwrap();
    std::fs::write(tmp.path().join("src/lib.rs"), "").unwrap();
    std::fs::write(tmp.path().join("README.md"), "").unwrap();

    if !fd_available() {
        return;
    }

    let result = tool.execute(r#"{"pattern": "src/*.rs"}"#).await.unwrap();
    assert!(!result.is_error, "got: {}", result.content);
    assert!(
        result.content.contains("main.rs"),
        "should find main.rs, got: {}",
        result.content
    );
    assert!(
        result.content.contains("lib.rs"),
        "should find lib.rs, got: {}",
        result.content
    );
    assert!(
        !result.content.contains("README.md"),
        "should not find README.md, got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_find_nested_path_segment_glob_matches() {
    let (tool, _ws, tmp) = test_find();
    std::fs::create_dir_all(tmp.path().join("nested")).unwrap();
    std::fs::write(tmp.path().join("nested/a.txt"), "").unwrap();
    std::fs::write(tmp.path().join("nested/b.log"), "").unwrap();
    std::fs::write(tmp.path().join("top.txt"), "").unwrap();

    if !fd_available() {
        return;
    }

    let result = tool
        .execute(r#"{"pattern": "nested/*.txt"}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "got: {}", result.content);
    assert!(
        result.content.contains("a.txt"),
        "should find a.txt, got: {}",
        result.content
    );
    assert!(
        !result.content.contains("b.log"),
        "should not find b.log, got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_find_exact_relative_path_matches() {
    let (tool, _ws, tmp) = test_find();
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::write(tmp.path().join("src/config.json"), "{}").unwrap();
    std::fs::write(tmp.path().join("src/other.json"), "{}").unwrap();
    std::fs::write(tmp.path().join("top.json"), "{}").unwrap();

    if !fd_available() {
        return;
    }

    let result = tool
        .execute(r#"{"pattern": "src/config.json"}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "got: {}", result.content);
    assert!(
        result.content.contains("config.json"),
        "should find config.json, got: {}",
        result.content
    );
    assert!(
        !result.content.contains("other.json"),
        "should not find other.json, got: {}",
        result.content
    );
    assert!(
        !result.content.contains("top.json"),
        "should not find top.json, got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_find_existing_double_star_pattern_unaffected() {
    // Existing **/*.rs patterns should continue to work after the fix.
    let (tool, _ws, tmp) = test_find();
    std::fs::create_dir_all(tmp.path().join("a/b/c")).unwrap();
    std::fs::write(tmp.path().join("a/b/c/deep.rs"), "").unwrap();

    if !fd_available() {
        return;
    }

    let result = tool.execute(r#"{"pattern": "**/*.rs"}"#).await.unwrap();
    assert!(!result.is_error, "got: {}", result.content);
    assert!(
        result.content.contains("deep.rs"),
        "should find deep.rs, got: {}",
        result.content
    );
}

#[test]
fn test_find_schema_includes_path_segment_example() {
    let (tool, _ws, _tmp) = test_find();
    let def = tool.definition();
    // Schema must show a path-segment glob example so agents know they work.
    assert!(
        def.parameters_schema.contains("src/*.rs")
            || def.parameters_schema.contains("nested/")
            || def.description.contains("src/"),
        "schema should demonstrate path-segment glob (e.g. 'src/*.rs'), got:\n{}",
        def.parameters_schema
    );
}
