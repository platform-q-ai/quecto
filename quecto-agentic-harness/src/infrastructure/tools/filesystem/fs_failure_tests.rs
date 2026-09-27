// #2189: every read/ls failure names the path, gives a plain reason and a
// next step, as a refusal with no "tool error" prefix (like edit's, #2193).

use super::*;
use crate::application::tools::ports::Tool;
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::filesystem::{EditTool, LsTool, ReadTool};
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

fn test_tools() -> (Arc<PathBuf>, Arc<Sandbox>, TempDir) {
    let tmp = TempDir::new().unwrap();
    let workspace = Arc::new(tmp.path().to_path_buf());
    let sandbox = Arc::new(Sandbox::new(Some(tmp.path().to_path_buf())));
    (workspace, sandbox, tmp)
}

async fn read(ws: &Arc<PathBuf>, sb: &Arc<Sandbox>, args: &str) -> ToolResult {
    let result = ReadTool::new(ws.clone(), sb.clone()).execute(args).await;
    result.unwrap_or_else(|error| panic!("read must refuse, not fail: {error}"))
}

async fn ls(ws: &Arc<PathBuf>, sb: &Arc<Sandbox>, path: &str) -> ToolResult {
    let args = serde_json::json!({ "path": path }).to_string();
    let result = LsTool::new(ws.clone(), sb.clone()).execute(&args).await;
    result.unwrap_or_else(|error| panic!("ls must refuse, not fail: {error}"))
}

fn refusal_text(result: &ToolResult) -> &str {
    assert!(
        result.is_error,
        "expected a refusal, got: {}",
        result.content
    );
    assert!(
        !result.content.contains("tool error") && !result.content.contains("os error"),
        "no raw error text: {}",
        result.content
    );
    &result.content
}

#[tokio::test]
async fn reading_a_directory_says_to_list_it() {
    let (ws, sb, tmp) = test_tools();
    std::fs::create_dir(tmp.path().join("dir")).unwrap();
    let result = read(&ws, &sb, r#"{"path": "dir/"}"#).await;
    assert_eq!(
        refusal_text(&result),
        "dir/ is a directory, not a file; list it with ls."
    );
}

#[tokio::test]
async fn reading_a_missing_file_names_where_it_was_looked_for() {
    let (ws, sb, tmp) = test_tools();
    std::fs::create_dir(tmp.path().join("A")).unwrap();
    let result = read(&ws, &sb, r#"{"path": "A/missing.txt"}"#).await;
    let full = tmp.path().join("A/missing.txt");
    assert_eq!(
        refusal_text(&result),
        format!(
            "file not found: A/missing.txt (looked for {}). Check the path, or list A with ls.",
            full.display()
        )
    );
}

#[tokio::test]
async fn an_absolute_missing_path_is_not_repeated() {
    let (ws, sb, tmp) = test_tools();
    let full = tmp.path().join("gone.txt");
    let args = serde_json::json!({ "path": full }).to_string();
    let result = read(&ws, &sb, &args).await;
    assert_eq!(
        refusal_text(&result),
        format!(
            "file not found: {}. Check the path, or list {} with ls.",
            full.display(),
            tmp.path().display()
        )
    );
}

#[tokio::test]
async fn a_path_through_a_file_says_a_part_is_a_file() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("regular"), "x").unwrap();
    let result = read(&ws, &sb, r#"{"path": "regular/inner"}"#).await;
    assert_eq!(
        refusal_text(&result),
        "cannot read regular/inner: a part of the path before its last name is a file, \
         not a directory. Check the path with ls."
    );
}

#[tokio::test]
async fn listing_a_missing_directory_names_the_parent_to_list() {
    let (ws, sb, _tmp) = test_tools();
    let result = ls(&ws, &sb, "A/missing").await;
    let text = refusal_text(&result);
    assert!(
        text.starts_with("directory not found: A/missing (looked for "),
        "{text}"
    );
    assert!(
        text.ends_with("). Check the path, or list A with ls."),
        "{text}"
    );
}

#[tokio::test]
async fn listing_a_file_says_to_read_it() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("regular"), "x").unwrap();
    let result = ls(&ws, &sb, "regular").await;
    assert_eq!(
        refusal_text(&result),
        "regular is a file, not a directory; read it with read."
    );
}

#[tokio::test]
async fn a_symbolic_link_loop_names_the_link_and_where_it_points() {
    let (ws, sb, tmp) = test_tools();
    std::os::unix::fs::symlink("loop", tmp.path().join("loop")).unwrap();
    let listed = ls(&ws, &sb, "loop").await;
    assert_eq!(
        refusal_text(&listed),
        "loop is a symbolic link in a loop: it points to loop, and following it leads back \
         to itself. Check it with bash, e.g. ls -l 'loop'"
    );
    let read_result = read(&ws, &sb, r#"{"path": "loop"}"#).await;
    assert!(
        refusal_text(&read_result).starts_with("loop is a symbolic link in a loop"),
        "{}",
        read_result.content
    );
}

#[tokio::test]
async fn a_loop_inside_the_path_is_named_as_one() {
    let (ws, sb, tmp) = test_tools();
    std::os::unix::fs::symlink("loop", tmp.path().join("loop")).unwrap();
    let result = ls(&ws, &sb, "loop/inner").await;
    assert_eq!(
        refusal_text(&result),
        "cannot list loop/inner: a directory in the path is a symbolic link that leads back \
         to itself, or a chain of more than 40 links. Check its parts with bash, e.g. ls -l \
         'loop/inner'"
    );
}

#[tokio::test]
async fn a_locked_directory_says_permission_denied() {
    use std::os::unix::fs::PermissionsExt;
    let (ws, sb, tmp) = test_tools();
    let locked = tmp.path().join("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let is_root = std::fs::read_dir(&locked).is_ok();
    let result = ls(&ws, &sb, "locked").await;
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    if is_root {
        return; // Running as root: permissions do not stop the listing.
    }
    assert_eq!(
        refusal_text(&result),
        "permission denied: cannot list locked. Check its permissions with bash, \
         e.g. ls -ld 'locked'"
    );
}

#[tokio::test]
async fn a_dangling_link_listed_says_what_it_points_to() {
    let (ws, sb, tmp) = test_tools();
    std::os::unix::fs::symlink("/nonexistent/target", tmp.path().join("link")).unwrap();
    let result = ls(&ws, &sb, "link").await;
    assert_eq!(
        refusal_text(&result),
        "link is a symbolic link to /nonexistent/target, which does not exist. \
         List the directory it should point to."
    );
}

#[tokio::test]
async fn bad_read_arguments_are_refusals_not_tool_errors() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("f.txt"), "a\nb\n").unwrap();
    for (args, wanted) in [
        (r#"{"path": "f.txt", "offset": 0}"#, "1-indexed"),
        (r#"{"path": "f.txt", "offset": 3.5}"#, "offset"),
        (r#"{"path": "f.txt", "limit": -1}"#, "limit"),
        (r#"{"path": "f.txt", "offset": 999}"#, "beyond end of file"),
    ] {
        let result = read(&ws, &sb, args).await;
        assert!(
            refusal_text(&result).contains(wanted),
            "{args}: {}",
            result.content
        );
    }
}

#[test]
fn every_other_error_names_the_path_and_keeps_the_system_text() {
    let error = std::io::Error::other("disk on fire");
    let text = explain_other(Access::List, "a b", &error);
    assert_eq!(
        text,
        "cannot list a b: disk on fire. Check the path with ls."
    );
}

#[test]
fn clearly_binary_bytes_are_named_binary_without_a_conversion_hint() {
    for bytes in [
        &b"\x7fELF\x02\x01\x01\xff"[..],
        &b"PK\x03\x04\xff\xfe"[..],
        &b"\x1f\x8b\x08\xff"[..],
        &b"text\0with a nul \xff"[..],
    ] {
        let text = not_utf8_text(Access::Read, "bin", bytes);
        assert!(
            text.starts_with("bin is not UTF-8 text: it is a binary file ("),
            "{text}"
        );
        assert!(text.contains("file 'bin'"), "{text}");
        assert!(!text.contains("iconv"), "{text}");
    }
    let text = not_utf8_text(Access::Read, "latin", b"caf\xe9\n");
    assert_eq!(
        text,
        "latin is not UTF-8 text (5B): binary, or text in another encoding. Inspect it with \
         bash, e.g. xxd 'latin' | head -n 40, or convert it, e.g. iconv -f latin1 -t utf-8 'latin'"
    );
    let edited = not_utf8_text(Access::Edit, "latin", b"caf\xe9\n");
    assert!(
        edited.contains("encoding, and edit changes UTF-8 text only. Inspect"),
        "{edited}"
    );
}

#[tokio::test]
async fn an_edit_of_a_path_through_a_file_is_explained() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("regular"), "x").unwrap();
    let args = r#"{"path": "regular/inner", "oldText": "a", "newText": "b"}"#;
    let result = EditTool::new(ws, sb).execute(args).await.unwrap();
    assert_eq!(
        refusal_text(&result),
        "cannot edit regular/inner: a part of the path before its last name is a file, \
         not a directory. Check the path with ls."
    );
}

/// #2189 review: UTF-16 text (it starts with a byte-order mark) is named as
/// such, with the conversion to make, not as binary for its NUL bytes.
#[test]
fn utf16_with_a_byte_order_mark_is_named_utf16() {
    for bytes in [&b"\xff\xfeh\0i\0"[..], &b"\xfe\xff\0h\0i"[..]] {
        let text = not_utf8_text(Access::Read, "u16.txt", bytes);
        assert_eq!(
            text,
            "u16.txt is not UTF-8 text: it is UTF-16 text (6B, it starts with a UTF-16 \
             byte-order mark). Convert it with bash, e.g. iconv -f UTF-16 -t UTF-8 'u16.txt'"
        );
    }
    let edited = not_utf8_text(Access::Edit, "u", b"\xff\xfeh\0");
    assert!(
        edited.contains("mark), and edit changes UTF-8 text only. Convert"),
        "{edited}"
    );
}

/// Re-review: a two-link cycle is named as a loop.
#[tokio::test]
async fn a_cycle_of_two_links_is_a_loop() {
    let (ws, sb, tmp) = test_tools();
    std::os::unix::fs::symlink("b", tmp.path().join("a")).unwrap();
    std::os::unix::fs::symlink("a", tmp.path().join("b")).unwrap();
    let result = read(&ws, &sb, r#"{"path": "a"}"#).await;
    assert!(
        refusal_text(&result).starts_with(
            "a is a symbolic link in a loop: it points to b, and following it leads back to \
             itself."
        ),
        "{}",
        result.content
    );
}

/// A chain of `count` links `l0` -> `l1` -> ... in `dir`, the last one
/// pointing at `end`.
fn chain(dir: &std::path::Path, count: usize, end: &str) {
    for i in 0..count {
        let next = match i + 1 == count {
            true => end.to_string(),
            false => format!("l{}", i + 1),
        };
        std::os::unix::fs::symlink(next, dir.join(format!("l{i}"))).unwrap();
    }
}

/// Re-review: an acyclic chain longer than the system follows is not
/// called a loop.
#[tokio::test]
async fn an_over_long_chain_of_links_is_not_called_a_loop() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("real.txt"), "x").unwrap();
    chain(tmp.path(), 41, "real.txt");
    let result = read(&ws, &sb, r#"{"path": "l0"}"#).await;
    assert_eq!(
        refusal_text(&result),
        "l0 is a chain of more than 40 symbolic links, which the system does not follow. \
         Point it at its target directly; check it with bash, e.g. ls -l 'l0'"
    );
}

/// Re-review: a dangling chain of 35 links is named as dangling, with the
/// target that does not exist.
#[tokio::test]
async fn a_long_dangling_chain_names_its_missing_target() {
    let (ws, sb, tmp) = test_tools();
    chain(tmp.path(), 35, "gone.txt");
    let result = read(&ws, &sb, r#"{"path": "l0"}"#).await;
    let gone = tmp.path().join("gone.txt");
    assert!(
        refusal_text(&result).starts_with(&format!(
            "l0 is a symbolic link to {}, which does not exist.",
            gone.display()
        )),
        "{}",
        result.content
    );
}
