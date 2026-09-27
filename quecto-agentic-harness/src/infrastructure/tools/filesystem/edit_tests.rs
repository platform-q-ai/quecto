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
async fn test_edit_replaces_unique_match() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("test.txt"), "hello world").unwrap();
    let tool = EditTool::new(ws, sb);
    let result = tool
        .execute(r#"{"path": "test.txt", "oldText": "hello", "newText": "goodbye"}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(
        result.content.contains("Successfully edited"),
        "expected diff output"
    );
    let content = std::fs::read_to_string(tmp.path().join("test.txt")).unwrap();
    assert_eq!(content, "goodbye world");
}

#[tokio::test]
async fn test_edit_legacy_old_new_params() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("f.txt"), "foo bar").unwrap();
    let tool = EditTool::new(ws, sb);
    let result = tool
        .execute(r#"{"path": "f.txt", "old": "foo", "new": "baz"}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
    let content = std::fs::read_to_string(tmp.path().join("f.txt")).unwrap();
    assert_eq!(content, "baz bar");
}

#[tokio::test]
async fn test_edit_substring_not_found() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("f.txt"), "hello world").unwrap();
    let tool = EditTool::new(ws, sb);
    let result = tool
        .execute(r#"{"path": "f.txt", "oldText": "xyz", "newText": "abc"}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("not found"));
}

#[tokio::test]
async fn test_edit_rejects_ambiguous_match() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("f.txt"), "aa aa").unwrap();
    let tool = EditTool::new(ws, sb);
    let result = tool
        .execute(r#"{"path": "f.txt", "oldText": "aa", "newText": "bb"}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("matches"));
}

#[tokio::test]
async fn test_edit_strips_bom() {
    let (ws, sb, tmp) = test_tools();
    let bom_content = "\u{FEFF}hello world";
    std::fs::write(tmp.path().join("f.txt"), bom_content).unwrap();
    let tool = EditTool::new(ws, sb);
    let result = tool
        .execute(r#"{"path": "f.txt", "oldText": "hello", "newText": "hi"}"#)
        .await
        .unwrap();
    assert!(
        !result.is_error,
        "BOM should be stripped: {}",
        result.content
    );
}

// --- Fuzzy content matching ---

#[tokio::test]
async fn test_edit_fuzzy_smart_single_quote() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("f.txt"), "it's a test").unwrap();
    let tool = EditTool::new(ws, sb);
    // oldText uses U+2019 RIGHT SINGLE QUOTATION MARK
    let result = tool
        .execute(r#"{"path":"f.txt","oldText":"it\u2019s a test","newText":"it's replaced"}"#)
        .await
        .unwrap();
    assert!(
        !result.is_error,
        "smart quote should fuzzy match: {}",
        result.content
    );
    let content = std::fs::read_to_string(tmp.path().join("f.txt")).unwrap();
    assert_eq!(content, "it's replaced");
}

#[tokio::test]
async fn test_edit_fuzzy_smart_double_quotes() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("f.txt"), "say \"hello\" now").unwrap();
    let tool = EditTool::new(ws, sb);
    // oldText uses U+201C/U+201D smart double quotes
    let result = tool
        .execute(
            "{\"path\":\"f.txt\",\"oldText\":\"say \\u201Chello\\u201D now\",\"newText\":\"say \\\"goodbye\\\" now\"}",
        )
        .await
        .unwrap();
    assert!(
        !result.is_error,
        "smart double quotes should fuzzy match: {}",
        result.content
    );
    let content = std::fs::read_to_string(tmp.path().join("f.txt")).unwrap();
    assert!(
        content.contains("goodbye"),
        "expected replacement, got: {}",
        content
    );
}

#[tokio::test]
async fn test_edit_fuzzy_preserves_non_edited_content() {
    // Fuzzy path must NOT fuzzy-rewrite the whole file; only the matched region changes.
    let (ws, sb, tmp) = test_tools();
    // Line 2 has smart quotes outside the edited region — must survive unchanged.
    let file = "say \"hello\" now\nline with \u{201C}preserved\u{201D} quotes\n";
    std::fs::write(tmp.path().join("f.txt"), file).unwrap();
    let tool = EditTool::new(ws, sb);
    let result = tool
        .execute(
            "{\"path\":\"f.txt\",\"oldText\":\"say \\u201Chello\\u201D now\",\"newText\":\"say hi now\"}",
        )
        .await
        .unwrap();
    assert!(
        !result.is_error,
        "fuzzy match should succeed: {}",
        result.content
    );
    let content = std::fs::read_to_string(tmp.path().join("f.txt")).unwrap();
    assert!(
        content.contains('\u{201C}') && content.contains('\u{201D}'),
        "smart quotes outside edited region must be preserved, got: {:?}",
        content
    );
    assert!(
        content.contains("say hi now"),
        "replacement must appear: {:?}",
        content
    );
}

#[tokio::test]
async fn test_edit_fuzzy_unicode_en_dash() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("f.txt"), "hello - world").unwrap();
    let tool = EditTool::new(ws, sb);
    // oldText uses U+2013 EN DASH
    let result = tool
        .execute(
            "{\"path\":\"f.txt\",\"oldText\":\"hello \\u2013 world\",\"newText\":\"replaced\"}",
        )
        .await
        .unwrap();
    assert!(
        !result.is_error,
        "en-dash should fuzzy match: {}",
        result.content
    );
    let content = std::fs::read_to_string(tmp.path().join("f.txt")).unwrap();
    assert_eq!(content, "replaced");
}

#[tokio::test]
async fn test_edit_fuzzy_trailing_whitespace() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("f.txt"), "hello\nworld").unwrap();
    let tool = EditTool::new(ws, sb);
    // oldText has trailing spaces on first line
    let result = tool
        .execute(r#"{"path":"f.txt","oldText":"hello   \nworld","newText":"replaced"}"#)
        .await
        .unwrap();
    assert!(
        !result.is_error,
        "trailing whitespace should fuzzy match: {}",
        result.content
    );
    let content = std::fs::read_to_string(tmp.path().join("f.txt")).unwrap();
    assert_eq!(content, "replaced");
}

// --- Line-ending preservation ---

#[tokio::test]
async fn test_edit_preserves_crlf_line_endings() {
    let (ws, sb, tmp) = test_tools();
    let crlf = "line1\r\nline2\r\nline3\r\n";
    std::fs::write(tmp.path().join("f.txt"), crlf).unwrap();
    let tool = EditTool::new(ws, sb);
    let result = tool
        .execute(r#"{"path":"f.txt","oldText":"line2","newText":"EDITED"}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "edit should succeed: {}", result.content);
    let bytes = std::fs::read(tmp.path().join("f.txt")).unwrap();
    assert!(
        bytes.windows(2).any(|w| w == b"\r\n"),
        "CRLF line endings should be preserved in written file"
    );
    let text = String::from_utf8(bytes).unwrap();
    assert!(
        text.contains("EDITED"),
        "replacement should appear in output"
    );
}

// --- BOM preservation ---

#[tokio::test]
async fn test_edit_preserves_bom_on_write() {
    let (ws, sb, tmp) = test_tools();
    let bom_content = "\u{FEFF}hello world";
    std::fs::write(tmp.path().join("f.txt"), bom_content).unwrap();
    let tool = EditTool::new(ws, sb);
    let result = tool
        .execute(r#"{"path":"f.txt","oldText":"hello","newText":"hi"}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "edit should succeed: {}", result.content);
    let bytes = std::fs::read(tmp.path().join("f.txt")).unwrap();
    assert!(
        bytes.starts_with(&[0xEF, 0xBB, 0xBF]),
        "UTF-8 BOM should be preserved on write, got: {:02X?}",
        &bytes[..bytes.len().min(6)]
    );
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("hi world"), "replacement should appear");
}

#[tokio::test]
async fn test_edit_rejects_noop_replacement() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("f.txt"), "hello world").unwrap();
    let tool = EditTool::new(ws, sb);
    let result = tool
        .execute(r#"{"path":"f.txt","oldText":"hello world","newText":"hello world"}"#)
        .await
        .unwrap();
    assert!(result.is_error, "no-op replacement should be an error");
    assert!(
        result.content.contains("identical"),
        "error should mention 'identical': {}",
        result.content
    );
}

#[tokio::test]
async fn test_edit_diff_context_4_lines() {
    let (ws, sb, tmp) = test_tools();
    // 10-line file; edit line 6 (f); context should include b,c,d,e (4 before)
    let content = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n";
    std::fs::write(tmp.path().join("f.txt"), content).unwrap();
    let tool = EditTool::new(ws, sb);
    let result = tool
        .execute(r#"{"path":"f.txt","oldText":"f","newText":"F"}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "edit should succeed: {}", result.content);
    // The diff should include 4 lines of context before the change
    assert!(
        result.content.contains("b"),
        "diff should contain 'b' as context"
    );
    assert!(
        result.content.contains("c"),
        "diff should contain 'c' as context"
    );
    assert!(
        result.content.contains("d"),
        "diff should contain 'd' as context"
    );
    assert!(
        result.content.contains("e"),
        "diff should contain 'e' as context"
    );
}

#[tokio::test]
async fn test_edit_diff_uses_minus_plus_markers() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("f.txt"), "line1\nline2\nline3\n").unwrap();
    let tool = EditTool::new(ws, sb);
    let result = tool
        .execute(r#"{"path":"f.txt","oldText":"line2","newText":"CHANGED"}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "edit should succeed: {}", result.content);
    // Quecto-style line-numbered diff: "-2 line2" and "+2 CHANGED"
    assert!(
        result.content.contains("-") && result.content.contains("line2"),
        "diff should contain removed line2: {}",
        result.content
    );
    assert!(
        result.content.contains("+") && result.content.contains("CHANGED"),
        "diff should contain added CHANGED: {}",
        result.content
    );
}

#[test]
fn test_plain_lf_without_bom_normalise_and_restore_are_identity() {
    let plain = "first\nsecond\n";
    assert_eq!(&*base_normalise(plain), plain);
    assert_eq!(&*restore_file_format(plain, LineEnding::Lf, false), plain);
}

#[test]
fn test_crlf_and_bom_paths_preserve_observable_format() {
    assert_eq!(&*base_normalise("first\r\nsecond\r\n"), "first\nsecond\n");
    assert_eq!(
        &*restore_file_format("first\nsecond\n", LineEnding::Crlf, false),
        "first\r\nsecond\r\n"
    );
    assert_eq!(
        &*restore_file_format("first\nsecond\n", LineEnding::Lf, true),
        "\u{FEFF}first\nsecond\n"
    );
}

#[tokio::test]
async fn test_edit_allows_file_at_size_limit() {
    let (ws, sb, tmp) = test_tools();
    let tool = EditTool::new(ws, sb);
    let mut content = String::from("TOKEN");
    content.push_str(&"a".repeat(1_048_576 - content.len()));
    std::fs::write(tmp.path().join("max-edit.txt"), content).unwrap();
    let result = tool
        .execute(r#"{"path": "max-edit.txt", "oldText": "TOKEN", "newText": "VALUE"}"#)
        .await
        .unwrap();
    assert!(
        !result.is_error,
        "file at size limit should edit: {}",
        result.content
    );
    let edited = std::fs::read_to_string(tmp.path().join("max-edit.txt")).unwrap();
    assert!(edited.starts_with("VALUE"));
    assert_eq!(edited.len(), 1_048_576);
}

/// #2193: too large to edit is a refusal with a way forward.
#[tokio::test]
async fn test_edit_rejects_oversized_file() {
    let (ws, sb, tmp) = test_tools();
    let tool = EditTool::new(ws, sb);
    let large_content = "a".repeat(1_048_577);
    std::fs::write(tmp.path().join("big-edit.txt"), large_content).unwrap();
    let result = tool
        .execute(r#"{"path": "big-edit.txt", "oldText": "a", "newText": "b"}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(
        result
            .content
            .contains("big-edit.txt is too large to edit (1.0MB; edit takes files up to 1.0MB)"),
        "{}",
        result.content
    );
    assert!(result.content.contains("sed"), "{}", result.content);
}

#[tokio::test]
async fn test_edit_empty_object_returns_actionable_error() {
    let (ws, sb, _tmp) = test_tools();
    let tool = EditTool::new(ws, sb);
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

#[test]
fn test_edit_description_includes_example() {
    let (ws, sb, _tmp) = test_tools();
    let tool = EditTool::new(ws, sb);
    let def = tool.definition();
    assert!(
        def.description.contains("Example"),
        "edit description should include Example, got: {}",
        def.description
    );
}

/// #2166: an empty oldText is refused as such, not as "not found".
#[tokio::test]
async fn an_empty_old_text_is_refused_as_empty() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("test.txt"), "hello").unwrap();
    let tool = EditTool::new(ws, sb);
    let result = tool
        .execute(r#"{"path": "test.txt", "oldText": "", "newText": "x"}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(
        result.content.contains("oldText must not be empty"),
        "{}",
        result.content
    );
}

/// #2183 review: text that normalises to nothing (a lone byte-order mark)
/// is refused as empty too.
#[tokio::test]
async fn an_old_text_of_only_a_bom_is_refused_as_empty() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("test.txt"), "hello").unwrap();
    let tool = EditTool::new(ws, sb);
    let result = tool
        .execute(
            &serde_json::json!({"path": "test.txt", "oldText": "\u{feff}", "newText": "x"})
                .to_string(),
        )
        .await
        .unwrap();
    assert!(
        result.content.contains("oldText must not be empty"),
        "{}",
        result.content
    );
}

async fn edit(tool: &EditTool, path: &str, old: &str, new: &str) -> ToolResult {
    let args = serde_json::json!({"path": path, "oldText": old, "newText": new});
    tool.execute(&args.to_string()).await.unwrap()
}

/// #2193 (B11): three matches are reported as three, with their lines.
#[tokio::test]
async fn every_match_is_counted_and_placed() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("triple.txt"), "triple triple triple\n").unwrap();
    let tool = EditTool::new(ws, sb);
    let result = edit(&tool, "triple.txt", "triple", "x").await;
    assert!(result.is_error);
    assert!(
        result
            .content
            .starts_with("oldText matches 3 times in triple.txt, all on line 1 —"),
        "{}",
        result.content
    );
    assert!(result.content.contains("surrounding"), "{}", result.content);
}

#[tokio::test]
async fn many_matches_name_the_lines_of_the_first_three() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("f.txt"), "x\ny\nx\nx\ny\nx\n").unwrap();
    let tool = EditTool::new(ws, sb);
    let result = edit(&tool, "f.txt", "x", "z").await;
    assert!(
        result
            .content
            .starts_with("oldText matches 4 times in f.txt, the first 3 on lines 1, 3 and 4 —"),
        "{}",
        result.content
    );
}

#[tokio::test]
async fn overlapping_matches_are_named_as_such() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("f.txt"), "aaa\n").unwrap();
    let tool = EditTool::new(ws, sb);
    let result = edit(&tool, "f.txt", "aa", "b").await;
    assert!(
        result
            .content
            .starts_with("oldText matches 2 times in f.txt (overlapping), all on line 1 —"),
        "{}",
        result.content
    );
}

/// #2193: line numbers count the file's lines, CRLF or not.
#[tokio::test]
async fn match_lines_are_file_lines_in_a_crlf_file() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("f.txt"), "\u{feff}a\r\nkey\r\nb\r\nkey\r\n").unwrap();
    let tool = EditTool::new(ws, sb);
    let result = edit(&tool, "f.txt", "key", "k").await;
    assert!(
        result
            .content
            .contains("2 times in f.txt, on lines 2 and 4"),
        "{}",
        result.content
    );
}

/// #2193 (B16): a missing file is named, with the tool that creates one.
#[tokio::test]
async fn a_missing_file_is_named_with_the_way_to_create_it() {
    let (ws, sb, tmp) = test_tools();
    let tool = EditTool::new(ws, sb);
    let result = edit(&tool, "sub/missing.txt", "a", "b").await;
    assert!(result.is_error);
    assert_eq!(
        result.content,
        format!(
            "file not found: sub/missing.txt (looked for {}). edit changes an existing file; \
             use write to create a new one.",
            tmp.path().join("sub/missing.txt").display()
        )
    );
}

#[tokio::test]
async fn a_dangling_symlink_is_named_as_one() {
    let (ws, sb, tmp) = test_tools();
    std::os::unix::fs::symlink("/nonexistent/target", tmp.path().join("link")).unwrap();
    let tool = EditTool::new(ws, sb);
    let result = edit(&tool, "link", "a", "b").await;
    assert!(result.is_error);
    assert!(
        result
            .content
            .contains("link is a symbolic link to /nonexistent/target"),
        "{}",
        result.content
    );
}

/// #2193 (B17): a binary file is refused as not UTF-8 text, as read does.
#[tokio::test]
async fn a_binary_file_is_refused_as_not_text() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("blob.dat"), [0u8, 159, 146, 150, 255, 1]).unwrap();
    let tool = EditTool::new(ws, sb);
    let result = edit(&tool, "blob.dat", "a", "b").await;
    assert!(result.is_error);
    assert!(
        result
            .content
            .starts_with("blob.dat is not UTF-8 text: it is a binary file (6B), and edit"),
        "{}",
        result.content
    );
    assert!(
        result.content.contains("xxd 'blob.dat'"),
        "{}",
        result.content
    );
    assert_eq!(
        std::fs::read(tmp.path().join("blob.dat")).unwrap(),
        [0u8, 159, 146, 150, 255, 1]
    );
}

#[tokio::test]
async fn a_directory_is_refused_as_one() {
    let (ws, sb, tmp) = test_tools();
    std::fs::create_dir(tmp.path().join("dir")).unwrap();
    let tool = EditTool::new(ws, sb);
    let result = edit(&tool, "dir", "a", "b").await;
    assert!(result.is_error);
    assert!(
        result.content.starts_with("dir is a directory"),
        "{}",
        result.content
    );
    assert!(result.content.contains("ls"), "{}", result.content);
}

/// #2193 (B13): a match with other indentation is pointed out.
#[tokio::test]
async fn an_indentation_mismatch_names_the_files_indentation() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("indent.txt"), "top\n\tindented text\n").unwrap();
    let tool = EditTool::new(ws, sb);
    let result = edit(&tool, "indent.txt", "    indented text", "x").await;
    assert!(result.is_error);
    assert_eq!(
        result.content,
        "oldText not found in indent.txt, but it matches at line 2 if indentation is \
         ignored; line 2 is indented with 1 tab in the file, 4 spaces in oldText. \
         Copy the indentation from the file."
    );
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("indent.txt")).unwrap(),
        "top\n\tindented text\n"
    );
}

/// With no near miss, "not found" still says what to do next.
#[tokio::test]
async fn not_found_says_to_copy_the_text_from_the_file() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("f.txt"), "hello\n").unwrap();
    let tool = EditTool::new(ws, sb);
    let result = edit(&tool, "f.txt", "goodbye", "x").await;
    assert_eq!(
        result.content,
        "oldText not found in f.txt. Read the file and copy oldText from it exactly, \
         whitespace included."
    );
}

/// #2193 review: a chain of links names the target that is missing, not
/// the next link.
#[tokio::test]
async fn a_chain_of_links_names_the_missing_target() {
    let (ws, sb, tmp) = test_tools();
    std::os::unix::fs::symlink("b", tmp.path().join("a")).unwrap();
    std::os::unix::fs::symlink("gone", tmp.path().join("b")).unwrap();
    let tool = EditTool::new(ws, sb);
    let result = edit(&tool, "a", "x", "y").await;
    let gone = tmp.path().canonicalize().unwrap().join("gone");
    assert!(
        result.content.starts_with(&format!(
            "a is a symbolic link to {}, which",
            gone.display()
        )),
        "{}",
        result.content
    );
}

/// #2193 review: a file that cannot be written is a refusal that names it.
#[tokio::test]
async fn a_read_only_file_is_refused_and_left_unchanged() {
    use std::os::unix::fs::PermissionsExt;
    let (ws, sb, tmp) = test_tools();
    let file = tmp.path().join("locked.txt");
    std::fs::write(&file, "hello\n").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o444)).unwrap();
    if std::fs::OpenOptions::new().write(true).open(&file).is_ok() {
        return; // Running as root: permissions do not stop the write.
    }
    let tool = EditTool::new(ws, sb);
    let result = edit(&tool, "locked.txt", "hello", "bye").await;
    assert!(result.is_error);
    assert!(
        result
            .content
            .starts_with("permission denied: cannot write locked.txt, so nothing was written."),
        "{}",
        result.content
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "hello\n");
}

/// #2193 review: a lone carriage return starts no line, as read shows it.
#[tokio::test]
async fn match_lines_ignore_a_lone_carriage_return() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("f.txt"), "a\rb\nkey\nc\rkey\n").unwrap();
    let tool = EditTool::new(ws, sb);
    let result = edit(&tool, "f.txt", "key", "k").await;
    assert!(
        result
            .content
            .contains("2 times in f.txt, on lines 2 and 3"),
        "{}",
        result.content
    );
    std::fs::write(tmp.path().join("g.txt"), "a\rb\n\tindented\n").unwrap();
    let result = edit(&tool, "g.txt", "  indented", "x").await;
    assert!(
        result.content.contains("matches at line 2 if indentation"),
        "{}",
        result.content
    );
}

/// #2193 review: the hint gives where the match starts and, apart, the
/// line whose indentation differs.
#[tokio::test]
async fn the_hint_names_the_match_start_and_the_differing_line() {
    let (ws, sb, tmp) = test_tools();
    let content = "fn a() {\n    keep();\n\tchange();\n}\n";
    std::fs::write(tmp.path().join("f.rs"), content).unwrap();
    let tool = EditTool::new(ws, sb);
    let result = edit(&tool, "f.rs", "fn a() {\n    keep();\n    change();", "x").await;
    assert!(
        result.content.contains(
            "matches at line 1 if indentation is ignored; line 3 is indented with 1 tab \
             in the file, 4 spaces in oldText"
        ),
        "{}",
        result.content
    );
}

/// #2193 review 2: an indentation hint that matches in several places lists
/// them and asks for more context.
#[tokio::test]
async fn an_ambiguous_indentation_hint_lists_every_line() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("f.txt"), "\tfoo\n\tfoo\n").unwrap();
    let tool = EditTool::new(ws, sb);
    let result = edit(&tool, "f.txt", "  foo", "x").await;
    assert_eq!(
        result.content,
        "oldText not found in f.txt, but it matches at lines 1 and 2 if indentation is \
         ignored; line 1 is indented with 1 tab in the file, 2 spaces in oldText. Copy the \
         indentation from the file and add surrounding lines to make it unique."
    );
}
