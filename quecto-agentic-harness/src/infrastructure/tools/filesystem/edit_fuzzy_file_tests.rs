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

// --- whitespace at the edges of oldText (#2191 review) ---
//
// The fuzzy needle drops whitespace at the edges of oldText, but newText
// still carries it. The match takes the file's whitespace run at that edge,
// so newText's whitespace replaces it instead of adding to it.

/// Trailing whitespace-only last line: the next line's indent is replaced.
#[tokio::test]
async fn a_whitespace_only_last_line_replaces_the_next_lines_indent() {
    assert_edit(
        "it\u{2019}s\n  bar\n",
        "it's\n  ",
        "done\n  ",
        "done\n  bar\n",
    )
    .await;
}

/// Trailing space mid-line: the file's space is replaced, not doubled.
#[tokio::test]
async fn a_trailing_space_mid_line_replaces_the_files_space() {
    assert_edit(
        "it\u{2019}s foo bar\n",
        "it's foo ",
        "it's baz ",
        "it's baz bar\n",
    )
    .await;
}

/// Mid-line, the match takes only as much of the file's whitespace run as
/// oldText ends with: the rest stays, as the exact path would leave it.
#[tokio::test]
async fn a_trailing_space_mid_line_takes_only_old_texts_whitespace() {
    assert_edit("it\u{2019}s foo \t bar\n", "it's foo ", "X ", "X \t bar\n").await;
    assert_edit("foo\u{2019}s  bar", "foo's ", "X ", "X  bar").await;
}

/// oldText's trailing whitespace that the file's run does not start with,
/// mid-line, is no match.
#[tokio::test]
async fn a_different_whitespace_run_mid_line_is_not_a_match() {
    let file = "it\u{2019}s foo\t bar\n";
    let (result, after) = edit_file(file, "it's foo ", "X ").await;
    assert!(result.content.contains("not found"), "{}", result.content);
    assert_eq!(after, file);
}

/// A shorter indent in oldText's last line keeps the rest of the file's.
#[tokio::test]
async fn a_shorter_indent_in_old_text_keeps_the_rest_of_the_files() {
    assert_edit(
        "it\u{2019}s\n        deep\n",
        "it's\n    ",
        "done\n    ",
        "done\n        deep\n",
    )
    .await;
}

/// Trailing whitespace at a line end is taken whole, whatever its length.
#[tokio::test]
async fn trailing_whitespace_at_a_line_end_is_taken_whole() {
    assert_edit("a\u{2019}\t  \nz\n", "a' ", "b ", "b \nz\n").await;
}

/// A trailing tab mid-line.
#[tokio::test]
async fn a_trailing_tab_mid_line_replaces_the_files_tab() {
    assert_edit("it\u{2019}s\tx\n", "it's\t", "Y\t", "Y\tx\n").await;
}

/// Trailing whitespace at end of line: the file's trailing run is replaced.
#[tokio::test]
async fn trailing_whitespace_at_line_end_replaces_the_files_run() {
    assert_edit("a\u{2019}  \n", "a'  ", "b  ", "b  \n").await;
}

/// oldText's trailing whitespace may stand for a line end with none.
#[tokio::test]
async fn trailing_whitespace_in_old_text_matches_a_bare_line_end() {
    assert_edit("a\u{2019}\nz\n", "a'  ", "b", "b\nz\n").await;
}

/// oldText ends in whitespace but the file has none there, mid-line: that is
/// not a match, so nothing is written.
#[tokio::test]
async fn trailing_whitespace_in_old_text_needs_whitespace_or_a_line_end() {
    let file = "it\u{2019}s foobar\n";
    let (result, after) = edit_file(file, "it's foo ", "X ").await;
    assert!(result.is_error, "{}", result.content);
    assert!(result.content.contains("not found"), "{}", result.content);
    assert_eq!(after, file);
}

/// A whitespace-only FIRST line of oldText takes the file's trailing run
/// before the matched newline.
#[tokio::test]
async fn a_whitespace_only_first_line_replaces_the_files_trailing_run() {
    assert_edit("pad  \nit\u{2019}s\n", "  \nit's", "  \nX", "pad  \nX\n").await;
}

/// Only matches whose edges fit count towards ambiguity.
#[tokio::test]
async fn only_matches_whose_edges_fit_are_counted() {
    assert_edit(
        "it\u{2019}s foobar\nit\u{2019}s foo bar\n",
        "it's foo ",
        "X ",
        "it\u{2019}s foobar\nX bar\n",
    )
    .await;
}

/// A whitespace-only first line of oldText must stand for whitespace in the
/// file (or a line that is empty): it never joins onto the end of a line.
#[tokio::test]
async fn a_whitespace_only_first_line_does_not_merge_lines() {
    let file = "  foo();\n    return x\u{2019};\n";
    let (result, after) = edit_file(file, "    \n    return x';", "    return y;").await;
    assert!(result.content.contains("not found"), "{}", result.content);
    assert_eq!(after, file);
}

/// ... but it does match an empty line.
#[tokio::test]
async fn a_whitespace_only_first_line_matches_an_empty_line() {
    assert_edit(
        "a\n\nreturn x\u{2019};\n",
        "  \nreturn x';",
        "\nreturn y;",
        "a\n\nreturn y;\n",
    )
    .await;
}
