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

// --- normalize_for_fuzzy_match unit tests ---

#[test]
fn test_fuzzy_normalise_smart_single_quotes() {
    // U+2018 U+2019 U+201A U+201B → '
    for ch in ['\u{2018}', '\u{2019}', '\u{201A}', '\u{201B}'] {
        let input = format!("it{ch}s");
        let result = normalize_for_fuzzy_match(&input);
        assert_eq!(result, "it's", "char U+{:04X} should become '", ch as u32);
    }
}

#[test]
fn test_fuzzy_normalise_smart_double_quotes() {
    // U+201C U+201D U+201E U+201F → "
    for ch in ['\u{201C}', '\u{201D}', '\u{201E}', '\u{201F}'] {
        let input = format!("{ch}hello{ch}");
        let result = normalize_for_fuzzy_match(&input);
        assert_eq!(
            result, "\"hello\"",
            "char U+{:04X} should become \"",
            ch as u32
        );
    }
}

#[test]
fn test_fuzzy_normalise_unicode_dashes() {
    // U+2010–U+2015, U+2212 → -
    for ch in [
        '\u{2010}', '\u{2011}', '\u{2012}', '\u{2013}', '\u{2014}', '\u{2015}', '\u{2212}',
    ] {
        let input = format!("a{ch}b");
        let result = normalize_for_fuzzy_match(&input);
        assert_eq!(result, "a-b", "char U+{:04X} should become -", ch as u32);
    }
}

#[test]
fn test_fuzzy_normalise_trailing_whitespace_per_line() {
    let input = "hello   \nworld  \n";
    let result = normalize_for_fuzzy_match(input);
    assert_eq!(result, "hello\nworld\n");
}

#[test]
fn test_fuzzy_normalise_special_spaces() {
    // NBSP and ideographic space → regular space
    let input = "a\u{00A0}b\u{3000}c";
    let result = normalize_for_fuzzy_match(input);
    assert_eq!(result, "a b c");
}

#[test]
fn test_fuzzy_normalise_strips_bom_and_crlf() {
    let input = "\u{FEFF}line1\r\nline2\r\n";
    let result = normalize_for_fuzzy_match(input);
    assert_eq!(result, "line1\nline2\n");
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

#[tokio::test]
async fn test_edit_rejects_oversized_file() {
    let (ws, sb, tmp) = test_tools();
    let tool = EditTool::new(ws, sb);
    let large_content = "a".repeat(1_048_577);
    std::fs::write(tmp.path().join("big-edit.txt"), large_content).unwrap();
    let result = tool
        .execute(r#"{"path": "big-edit.txt", "oldText": "a", "newText": "b"}"#)
        .await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("exceeds maximum allowed size")
    );
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
