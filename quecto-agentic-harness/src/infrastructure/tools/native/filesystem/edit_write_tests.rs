// #2243: an edit's file is replaced whole or not at all, keeping its mode
// and its links; the output is byte-for-byte what the edit makes.

use super::*;
use crate::infrastructure::security::sandbox::Sandbox;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use tempfile::TempDir;

fn test_tools() -> (Arc<PathBuf>, Arc<Sandbox>, TempDir) {
    let tmp = TempDir::new().unwrap();
    let workspace = Arc::new(tmp.path().to_path_buf());
    let sandbox = Arc::new(Sandbox::new(Some(tmp.path().to_path_buf())));
    (workspace, sandbox, tmp)
}

async fn edit(tool: &EditTool, path: &str, old: &str, new: &str) -> ToolResult {
    let args = serde_json::json!({ "path": path, "oldText": old, "newText": new });
    tool.execute(&args.to_string()).await.unwrap()
}

#[tokio::test]
async fn a_successful_edit_writes_exactly_the_edited_bytes_by_a_rename() {
    let (ws, sb, tmp) = test_tools();
    let file = tmp.path().join("f.rs");
    let before = "fn main() {\n    println!(\"hi\");\n}\n";
    std::fs::write(&file, before).unwrap();
    let inode = std::fs::metadata(&file).unwrap().ino();
    let result = edit(&EditTool::new(ws, sb), "f.rs", "\"hi\"", "\"bye\"").await;
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(
        std::fs::read(&file).unwrap(),
        before.replace("\"hi\"", "\"bye\"").into_bytes()
    );
    assert_ne!(std::fs::metadata(&file).unwrap().ino(), inode, "renamed in");
    assert!(!result.content.contains("[note:"), "{}", result.content);
    let names: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().collect();
    assert_eq!(names.len(), 1, "no temporary file is left");
}

#[tokio::test]
async fn the_mode_and_a_symbolic_link_are_kept() {
    let (ws, sb, tmp) = test_tools();
    std::fs::create_dir(tmp.path().join("real")).unwrap();
    let target = tmp.path().join("real/script.sh");
    std::fs::write(&target, "echo old\n").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::symlink("real/script.sh", tmp.path().join("script.sh")).unwrap();
    let result = edit(&EditTool::new(ws, sb), "script.sh", "old", "new").await;
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "echo new\n");
    let link = std::fs::symlink_metadata(tmp.path().join("script.sh")).unwrap();
    assert!(link.file_type().is_symlink());
    assert_eq!(std::fs::metadata(&target).unwrap().mode() & 0o7777, 0o755);
}

#[tokio::test]
async fn a_hard_linked_file_is_edited_in_place_and_the_diff_says_so() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("a.txt"), "one\ntwo\n").unwrap();
    std::fs::hard_link(tmp.path().join("a.txt"), tmp.path().join("b.txt")).unwrap();
    let result = edit(&EditTool::new(ws, sb), "a.txt", "two", "2").await;
    assert!(!result.is_error, "{}", result.content);
    assert!(
        result.content.ends_with(
            "\n[note: a.txt has 2 hard links, so it was written in place to keep them, \
             not atomically]"
        ),
        "{}",
        result.content
    );
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("b.txt")).unwrap(),
        "one\n2\n"
    );
}
