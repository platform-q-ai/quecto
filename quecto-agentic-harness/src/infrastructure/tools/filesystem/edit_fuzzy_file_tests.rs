//! #2191: the fuzzy fallback must splice the ORIGINAL file at the place it
//! matched. These tests put the curly quotes and trailing whitespace in the
//! FILE (not only in `oldText`) and assert the exact bytes written.

use super::*;
use crate::infrastructure::security::sandbox::Sandbox;
use tempfile::TempDir;

fn tool_in_tmp() -> (EditTool, TempDir) {
    let tmp = TempDir::new().unwrap();
    let workspace = Arc::new(tmp.path().to_path_buf());
    let sandbox = Arc::new(Sandbox::new(Some(tmp.path().to_path_buf())));
    (EditTool::new(workspace, sandbox), tmp)
}

/// Write `file`, run one edit, return the result and the bytes on disk.
async fn edit_file(file: &str, old: &str, new: &str) -> (ToolResult, String) {
    let (tool, tmp) = tool_in_tmp();
    let path = tmp.path().join("f.txt");
    std::fs::write(&path, file).unwrap();
    let args = serde_json::json!({"path": "f.txt", "oldText": old, "newText": new});
    let result = tool.execute(&args.to_string()).await.unwrap();
    let after = std::fs::read_to_string(&path).unwrap();
    (result, after)
}

async fn assert_edit(file: &str, old: &str, new: &str, expected: &str) {
    let (result, after) = edit_file(file, old, new).await;
    assert!(!result.is_error, "edit failed: {}", result.content);
    assert_eq!(after, expected, "file {file:?}, oldText {old:?}");
}

/// QA X1: trailing spaces inside the match are replaced with it.
#[tokio::test]
async fn trailing_spaces_inside_the_match_are_replaced_with_it() {
    assert_edit("foo  \nbar\nbaz\n", "foo\nbar", "X", "X\nbaz\n").await;
}

/// QA X4: trailing spaces BEFORE the match stay, and the match is replaced.
#[tokio::test]
async fn trailing_spaces_before_the_match_are_left_alone() {
    assert_edit(
        "pad  \nfoo  \nbar\nend\n",
        "foo\nbar",
        "X",
        "pad  \nX\nend\n",
    )
    .await;
}

/// QA X2: a curly quote in the file matches a straight one in `oldText`.
#[tokio::test]
async fn a_curly_quote_in_the_file_is_replaced_whole() {
    assert_edit("it\u{2019}s here\n", "it's here", "done", "done\n").await;
}

/// QA X3: a multi-byte character before the match used to panic.
#[tokio::test]
async fn curly_quotes_before_the_match_do_not_panic() {
    assert_edit(
        "\u{2019}\u{2019}ab\u{2019}\n",
        "b'",
        "Z",
        "\u{2019}\u{2019}aZ\n",
    )
    .await;
}

/// Trailing whitespace right AFTER the match is not part of it: the file's
/// other text stays byte-identical.
#[tokio::test]
async fn trailing_spaces_after_the_match_are_left_alone() {
    assert_edit("it\u{2019}s  \nnext\n", "it's", "X", "X  \nnext\n").await;
}

/// A match that ends on a newline takes that newline, and only it.
#[tokio::test]
async fn a_match_ending_on_a_newline_keeps_the_next_line() {
    assert_edit("a\u{2013}b  \nc\n", "a-b\n", "", "c\n").await;
}

/// Special characters after the match survive untouched.
#[tokio::test]
async fn text_after_the_match_is_byte_identical() {
    assert_edit(
        "\u{201C}q\u{201D}  \n\u{2014} tail\u{00A0}\u{2019}  \n",
        "\"q\"",
        "Q",
        "Q  \n\u{2014} tail\u{00A0}\u{2019}  \n",
    )
    .await;
}

/// CRLF files keep CRLF, and the match lands in the right place.
#[tokio::test]
async fn a_crlf_file_is_spliced_in_place_and_keeps_crlf() {
    assert_edit(
        "pad  \r\nfoo  \r\nbar\r\nend\r\n",
        "foo\r\nbar",
        "X",
        "pad  \r\nX\r\nend\r\n",
    )
    .await;
}

/// A BOM is kept, and the splice is measured after it.
#[tokio::test]
async fn a_bom_file_is_spliced_in_place() {
    assert_edit("\u{FEFF}\u{2019}x  \ny\n", "'x\ny", "Z", "\u{FEFF}Z\n").await;
}

/// The fuzzy fallback refuses more than one match, as the exact path does.
#[tokio::test]
async fn an_ambiguous_fuzzy_match_is_refused_and_nothing_is_written() {
    let file = "it\u{2019}s  \nit\u{2018}s\n";
    let (result, after) = edit_file(file, "it's", "X").await;
    assert!(result.is_error, "{}", result.content);
    assert!(
        result.content.contains("matches 2 times"),
        "{}",
        result.content
    );
    assert_eq!(after, file);
}

/// The diff shows the edit that was actually written.
#[tokio::test]
async fn the_diff_reports_the_real_edit() {
    let (result, after) = edit_file("pad  \nfoo  \nbar\nend\n", "foo\nbar", "X").await;
    assert_eq!(after, "pad  \nX\nend\n");
    assert!(
        result.content.contains("-2 foo  \n-3 bar\n+2 X"),
        "{}",
        result.content
    );
    assert!(!result.content.contains("-1 pad"), "{}", result.content);
}

/// The splice refuses, never panics on, a range off a character boundary,
/// past the end, or back to front.
#[test]
fn splice_refuses_ranges_it_cannot_cut_cleanly() {
    let text = "a\u{2019}b";
    assert_eq!(splice(text, 1..4, "'").as_deref(), Some("a'b"));
    assert_eq!(splice(text, 2..4, "x"), None);
    assert_eq!(splice(text, 1..3, "x"), None);
    assert_eq!(splice(text, 4..9, "x"), None);
    #[expect(clippy::reversed_empty_ranges, reason = "a back-to-front range")]
    let reversed = 4..1;
    assert_eq!(splice(text, reversed, "x"), None);
}
