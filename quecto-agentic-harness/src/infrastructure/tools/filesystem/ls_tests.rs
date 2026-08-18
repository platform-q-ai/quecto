use super::*;
use crate::infrastructure::security::sandbox::Sandbox;
use tempfile::TempDir;

fn test_tools() -> (Arc<PathBuf>, Arc<Sandbox>, TempDir) {
    let tmp = TempDir::new().unwrap();
    let workspace = Arc::new(tmp.path().to_path_buf());
    let sandbox = Arc::new(Sandbox::new(Some(tmp.path().to_path_buf())));
    (workspace, sandbox, tmp)
}

#[tokio::test]
async fn test_ls_lists_files_and_dirs() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("a.txt"), "").unwrap();
    std::fs::create_dir(tmp.path().join("subdir")).unwrap();
    let tool = LsTool::new(ws, sb);
    let result = tool.execute(r#"{}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("a.txt"));
    assert!(result.content.contains("subdir/"));
}

#[tokio::test]
async fn test_ls_defaults_to_workspace() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("file.txt"), "").unwrap();
    let tool = LsTool::new(ws, sb);
    let result = tool.execute(r#"{}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("file.txt"));
}

#[tokio::test]
async fn test_ls_subdirectory() {
    let (ws, sb, tmp) = test_tools();
    std::fs::create_dir(tmp.path().join("sub")).unwrap();
    std::fs::write(tmp.path().join("sub/inner.txt"), "x").unwrap();
    let tool = LsTool::new(ws, sb);
    let result = tool.execute(r#"{"path": "sub"}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("inner.txt"));
}

#[tokio::test]
async fn test_ls_allows_path_outside_workspace() {
    let (ws, sb, _tmp) = test_tools();
    let outside = TempDir::new().unwrap();
    std::fs::write(outside.path().join("outside.txt"), "ok").unwrap();
    let tool = LsTool::new(ws, sb);
    let result = tool
        .execute(&format!(r#"{{"path": "{}"}}"#, outside.path().display()))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert!(result.content.contains("outside.txt"));
}

// --- Quecto compatibility ---

#[tokio::test]
async fn test_ls_empty_directory_message() {
    let (ws, sb, _tmp) = test_tools();
    let tool = LsTool::new(ws, sb);
    let result = tool.execute(r#"{}"#).await.unwrap();
    assert!(!result.is_error);
    assert_eq!(result.content, "(empty directory)");
}

#[tokio::test]
async fn test_ls_case_insensitive_sort() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("Makefile"), "").unwrap();
    std::fs::write(tmp.path().join("app.rs"), "").unwrap();
    std::fs::write(tmp.path().join("Zoo.rs"), "").unwrap();
    let tool = LsTool::new(ws, sb);
    let result = tool.execute(r#"{}"#).await.unwrap();
    assert!(!result.is_error);
    let lines: Vec<&str> = result.content.lines().collect();
    // Case-insensitive: app.rs < Makefile < Zoo.rs
    let idx_app = lines.iter().position(|&l| l == "app.rs").unwrap();
    let idx_make = lines.iter().position(|&l| l == "Makefile").unwrap();
    let idx_zoo = lines.iter().position(|&l| l == "Zoo.rs").unwrap();
    assert!(
        idx_app < idx_make && idx_make < idx_zoo,
        "expected app.rs < Makefile < Zoo.rs, got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_ls_limit_parameter() {
    let (ws, sb, tmp) = test_tools();
    for i in 0..20 {
        std::fs::write(tmp.path().join(format!("file_{:04}.txt", i)), "").unwrap();
    }
    let tool = LsTool::new(ws, sb);
    let result = tool.execute(r#"{"limit": 5}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(
        result.content.contains("5 entries limit reached"),
        "expected limit notice, got: {}",
        result.content
    );
    assert!(
        result.content.contains("limit=10"),
        "expected suggested limit, got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_ls_default_limit_is_500() {
    let (ws, sb, tmp) = test_tools();
    for i in 0..600 {
        std::fs::write(tmp.path().join(format!("file_{:04}.txt", i)), "").unwrap();
    }
    let tool = LsTool::new(ws, sb);
    let result = tool.execute(r#"{}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(
        result.content.contains("500 entries limit reached"),
        "expected 500 limit notice, got: {}",
        &result.content[result.content.len().saturating_sub(200)..]
    );
}

#[tokio::test]
async fn test_ls_float_limit() {
    let (ws, sb, tmp) = test_tools();
    for i in 0..20 {
        std::fs::write(tmp.path().join(format!("file_{:04}.txt", i)), "").unwrap();
    }
    let tool = LsTool::new(ws, sb);
    let result = tool.execute(r#"{"limit": 5.0}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(
        result.content.contains("5 entries limit reached"),
        "expected limit notice, got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_ls_invalid_json_is_tool_error() {
    let (ws, sb, _tmp) = test_tools();
    let tool = LsTool::new(ws, sb);
    let result = tool.execute(r#"{"#).await.unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("invalid JSON arguments"));
}

#[tokio::test]
async fn test_ls_float_limit_rounds_and_clamps_to_minimum() {
    let (ws, sb, tmp) = test_tools();
    for name in ["a", "b", "c"] {
        std::fs::write(tmp.path().join(name), "").unwrap();
    }
    let tool = LsTool::new(ws, sb);
    let result = tool.execute(r#"{"limit": 0.4}"#).await.unwrap();
    assert!(!result.is_error);
    let first = result.content.lines().next().unwrap();
    assert!(["a", "b", "c"].contains(&first));
    assert!(result.content.contains("1 entries limit reached"));
    assert!(result.content.contains("limit=2"));
}

#[test]
fn test_ls_description_includes_example() {
    let (ws, sb, _tmp) = test_tools();
    let tool = LsTool::new(ws, sb);
    let def = tool.definition();
    assert!(
        def.description.contains("Example"),
        "ls description should include Example, got: {}",
        def.description
    );
}
