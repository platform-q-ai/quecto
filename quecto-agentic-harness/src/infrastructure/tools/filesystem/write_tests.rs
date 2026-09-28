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

async fn write(tool: &WriteTool, path: &str, content: &str) -> ToolResult {
    let args = serde_json::json!({ "path": path, "content": content });
    tool.execute(&args.to_string()).await.unwrap()
}

/// #2243: an existing file is replaced whole, keeping its mode; a link is
/// kept and its target written.
#[tokio::test]
async fn a_file_is_replaced_whole_keeping_its_mode_and_links() {
    use std::os::unix::fs::PermissionsExt;
    let (ws, sb, tmp) = test_tools();
    let target = tmp.path().join("run.sh");
    std::fs::write(&target, "old").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o750)).unwrap();
    std::os::unix::fs::symlink("run.sh", tmp.path().join("link.sh")).unwrap();
    let tool = WriteTool::new(ws, sb);

    let result = write(&tool, "link.sh", "new").await;
    assert_eq!(result.content, "Successfully wrote 3 bytes to link.sh");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
    let meta = std::fs::symlink_metadata(tmp.path().join("link.sh")).unwrap();
    assert!(meta.file_type().is_symlink());
    let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode, 0o750);
}

/// #2243: a hard-linked file is written in place, and the result says so.
#[tokio::test]
async fn a_hard_linked_file_is_written_in_place_and_said_so() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("a.txt"), "old").unwrap();
    std::fs::hard_link(tmp.path().join("a.txt"), tmp.path().join("b.txt")).unwrap();
    let result = write(&WriteTool::new(ws, sb), "a.txt", "new").await;
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(
        result.content,
        "Successfully wrote 3 bytes to a.txt\n[note: a.txt has 2 hard links, so it was \
         written in place to keep them, not atomically]"
    );
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("b.txt")).unwrap(),
        "new"
    );
}

/// #2243: a failed write is a refusal that names the file, not a tool error.
#[tokio::test]
async fn writing_over_a_directory_is_refused_by_name() {
    let (ws, sb, tmp) = test_tools();
    std::fs::create_dir(tmp.path().join("dir")).unwrap();
    let result = write(&WriteTool::new(ws, sb), "dir", "x").await;
    assert!(result.is_error);
    assert_eq!(
        result.content,
        "dir is a directory, so nothing was written. Give the path of a file."
    );
}
