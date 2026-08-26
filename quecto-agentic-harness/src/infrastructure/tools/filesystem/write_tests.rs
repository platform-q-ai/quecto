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
async fn test_write_creates_parent_dirs_and_success_message() {
    let (ws, sb, tmp) = test_tools();
    let tool = WriteTool::new(ws, sb);
    let result = tool
        .execute(r#"{"path": "sub/dir/file.txt", "content": "nested"}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(tmp.path().join("sub/dir/file.txt").exists());
    assert!(result.content.contains("bytes"));
}

#[tokio::test]
async fn test_write_empty_object_returns_actionable_error() {
    let (ws, sb, _tmp) = test_tools();
    let tool = WriteTool::new(ws, sb);
    let result = tool.execute("{}").await.unwrap();
    assert!(result.is_error, "expected error, got: {}", result.content);
    assert!(
        result.content.contains("path"),
        "should mention 'path', got: {}",
        result.content
    );
    assert!(
        result.content.contains("Example"),
        "should include example, got: {}",
        result.content
    );
}

#[tokio::test]
async fn write_rejects_malformed_json() {
    let (ws, sb, _tmp) = test_tools();
    let tool = WriteTool::new(ws, sb);
    let result = tool.execute("{not json").await.unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("invalid JSON arguments"));
}

#[tokio::test]
async fn write_requires_string_content() {
    let (ws, sb, _tmp) = test_tools();
    let tool = WriteTool::new(ws, sb);
    let result = tool.execute(r#"{"path":"out.txt"}"#).await.unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("content"));
}

#[test]
fn test_write_description_includes_example() {
    let (ws, sb, _tmp) = test_tools();
    let tool = WriteTool::new(ws, sb);
    let def = tool.definition();
    assert!(
        def.description.contains("Example"),
        "write description should include Example, got: {}",
        def.description
    );
}
