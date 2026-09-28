// #2242: an edit changes only the matched span. Bytes outside it, line
// endings and lone carriage returns included, are written back as they were,
// and the diff's line numbers count the file's own line breaks.

use super::*;
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::filesystem::edit_match::base_mapped;
use tempfile::TempDir;

fn test_tools() -> (Arc<PathBuf>, Arc<Sandbox>, TempDir) {
    let tmp = TempDir::new().unwrap();
    let workspace = Arc::new(tmp.path().to_path_buf());
    let sandbox = Arc::new(Sandbox::new(Some(tmp.path().to_path_buf())));
    (workspace, sandbox, tmp)
}

/// Edit `before` (the file's bytes) replacing `old` with `new`: the file's
/// bytes after, and the tool's result.
async fn edited(before: &str, old: &str, new: &str) -> (String, ToolResult) {
    let (ws, sb, tmp) = test_tools();
    let file = tmp.path().join("f.txt");
    std::fs::write(&file, before).unwrap();
    let args = serde_json::json!({ "path": "f.txt", "oldText": old, "newText": new });
    let result = EditTool::new(ws, sb)
        .execute(&args.to_string())
        .await
        .unwrap();
    (std::fs::read_to_string(&file).unwrap(), result)
}

#[tokio::test]
async fn lone_carriage_returns_outside_the_edit_survive() {
    let (after, result) = edited("progress 1%\rprogress 50%\rdone\nx\n", "x", "y").await;
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(after, "progress 1%\rprogress 50%\rdone\ny\n");
}

#[tokio::test]
async fn diff_line_numbers_count_the_files_own_line_breaks() {
    let (_, result) = edited("p\rq\rr\nold\n", "old", "new").await;
    assert!(!result.is_error, "{}", result.content);
    assert!(result.content.contains("-2 old"), "{}", result.content);
    assert!(result.content.contains("+2 new"), "{}", result.content);
    assert!(result.content.contains(" 1 p\rq\rr"), "{}", result.content);
}

#[tokio::test]
async fn a_crlf_file_keeps_crlf_and_new_lines_take_it() {
    let (after, result) = edited("a\r\nb\r\nc\r\n", "b", "B\nB2").await;
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(after, "a\r\nB\r\nB2\r\nc\r\n");
    assert!(result.content.contains("+3 B2"), "{}", result.content);
    assert!(!result.content.contains('\r'), "{}", result.content);
}

#[tokio::test]
async fn a_mixed_file_keeps_every_line_ending_outside_the_edit() {
    let (after, _) = edited("a\r\nb\nc\r\nd\n", "c", "C").await;
    assert_eq!(after, "a\r\nb\nC\r\nd\n");
}

#[tokio::test]
async fn a_match_across_crlf_breaks_replaces_them_whole() {
    let (after, _) = edited("a\r\nb\r\nc\r\n", "a\nb\n", "x\n").await;
    assert_eq!(after, "x\r\nc\r\n");
    let (after, _) = edited("a\r\nb\r\nc\r\n", "b\r\nc", "z").await;
    assert_eq!(after, "a\r\nz\r\n");
}

/// A lone `\r` inside the matched span is part of what is replaced (it
/// matches a line break in oldText, written `\n`, `\r\n` or `\r`), and
/// newText's line breaks are written with the file's line ending.
#[tokio::test]
async fn a_lone_carriage_return_inside_the_span_goes_with_it() {
    let (after, _) = edited("a\rb\nc\rd\n", "a\nb", "a b").await;
    assert_eq!(after, "a b\nc\rd\n");
    let (after, _) = edited("a\rb\nc\rd\n", "a\rb", "a\rB").await;
    assert_eq!(after, "a\nB\nc\rd\n");
}

#[tokio::test]
async fn a_bom_and_crlf_are_kept() {
    let (after, _) = edited("\u{FEFF}a\r\nb\r\n", "b", "c").await;
    assert_eq!(after, "\u{FEFF}a\r\nc\r\n");
}

#[tokio::test]
async fn a_fuzzy_match_splices_into_the_original_bytes() {
    let (after, result) = edited("x\r\n\u{201C}quoted\u{201D}  \r\ny\r", "\"quoted\"", "q").await;
    assert!(!result.is_error, "{}", result.content);
    // Trailing whitespace after the match is the file's, and stays.
    assert_eq!(after, "x\r\nq  \r\ny\r");
}

#[tokio::test]
async fn an_edit_that_changes_only_a_line_ending_is_not_a_no_op() {
    let (after, result) = edited("a\rb\n", "a\nb", "a\nb").await;
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(after, "a\nb\n");
    let (after, result) = edited("a\r\nb\r\n", "a\nb", "a\r\nb").await;
    assert!(result.is_error, "{}", result.content);
    assert_eq!(after, "a\r\nb\r\n");
}

#[test]
fn the_base_map_points_each_byte_at_the_character_that_made_it() {
    let raw = "\u{FEFF}a\r\nb\rc\u{e9}";
    let mapped = base_mapped(raw).unwrap();
    assert_eq!(mapped.text, "a\nb\nc\u{e9}");
    assert_eq!(mapped.text, base_normalise(raw));
    // "a" is after the BOM; the CRLF's "\n" covers both bytes; a lone "\r"
    // covers itself.
    assert_eq!(mapped.original_range(raw, 0..1), Some(3..4));
    assert_eq!(mapped.original_range(raw, 1..2), Some(4..6));
    assert_eq!(mapped.original_range(raw, 0..3), Some(3..7));
    assert_eq!(mapped.original_range(raw, 3..4), Some(7..8));
    assert_eq!(mapped.original_range(raw, 4..7), Some(8..11));
    assert_eq!(mapped.original_range(raw, 5..7), Some(9..11));
    assert_eq!(mapped.original_range(raw, 6..7), None, "mid-character");
}

#[test]
fn view_offsets_drop_the_bom_and_each_crlfs_carriage_return() {
    let raw = "\u{FEFF}a\r\nb\rc\r\n";
    assert_eq!(line_view(raw), "a\nb\rc\n");
    assert_eq!(view_offset(raw, 3), 0);
    assert_eq!(view_offset(raw, 4), 1);
    assert_eq!(view_offset(raw, 6), 2);
    assert_eq!(view_offset(raw, 8), 4);
    assert_eq!(view_offset(raw, raw.len()), line_view(raw).len());
}

/// #2242 review: newText's line breaks take the ending of the lines it
/// replaces, not of the file's first line: an LF region of a mixed file
/// stays LF.
#[tokio::test]
async fn new_lines_take_the_ending_of_the_matched_lines() {
    let (after, _) = edited("a\r\nb\r\nx\ny\nz\n", "x\ny", "x\nY").await;
    assert_eq!(after, "a\r\nb\r\nx\nY\nz\n");
    let (after, _) = edited("a\nb\nx\r\ny\r\n", "x\r\ny", "X\nY").await;
    assert_eq!(after, "a\nb\nX\r\nY\r\n");
    // A match within one line takes the ending of the line it is on.
    let (after, _) = edited("a\r\nb\nc\n", "b", "b1\nb2").await;
    assert_eq!(after, "a\r\nb1\nb2\nc\n");
    let (after, _) = edited("a\nb\r\nc\n", "b", "b1\nb2").await;
    assert_eq!(after, "a\nb1\r\nb2\r\nc\n");
}

/// #2242 review: newText equal to oldText in an LF region of a CRLF-first
/// file changes nothing and is refused as such.
#[tokio::test]
async fn a_same_text_edit_in_a_mixed_file_is_a_no_op() {
    let before = "a\r\nx\ny\n";
    let (after, result) = edited(before, "x\ny\n", "x\ny\n").await;
    assert!(result.is_error, "{}", result.content);
    assert!(
        result.content.starts_with("No changes made"),
        "{}",
        result.content
    );
    assert_eq!(after, before);
}

/// Known edge (documented): deleting the text between a kept lone `\r`
/// and a line break joins them into one `\r\n`.
#[tokio::test]
async fn a_kept_lone_carriage_return_can_join_the_next_line_break() {
    let (after, result) = edited("a\rX\nb", "X", "").await;
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(after, "a\r\nb");
}

#[test]
fn the_line_ending_of_a_span() {
    use crate::infrastructure::tools::filesystem::edit_bytes::{LineEnding, span_line_ending};
    assert_eq!(span_line_ending("a\r\nb\n", 0..3), LineEnding::Crlf);
    assert_eq!(span_line_ending("a\r\nb\n", 3..5), LineEnding::Lf);
    assert_eq!(span_line_ending("a\r\nb\n", 3..4), LineEnding::Lf);
    assert_eq!(span_line_ending("a\r\nb\n", 0..1), LineEnding::Crlf);
    // The last line, with no break of its own: the line before's.
    assert_eq!(span_line_ending("a\r\nb", 3..4), LineEnding::Crlf);
    assert_eq!(span_line_ending("a\nb", 2..3), LineEnding::Lf);
    // One line and no break at all: LF.
    assert_eq!(span_line_ending("ab", 0..1), LineEnding::Lf);
}

/// Round 2 L3: each replaced line keeps its own ending; lines added beyond
/// the old ones take the ending of the last replaced line.
#[tokio::test]
async fn each_replaced_line_keeps_its_own_ending() {
    let (after, result) = edited("a\nx\r\ny\nz\n", "x\r\ny\n", "x\ny2\n").await;
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(after, "a\nx\r\ny2\nz\n");
    let (after, _) = edited("x\ny\r\nz", "x\ny", "x\ny\nw").await;
    assert_eq!(after, "x\ny\r\nw\r\nz");
    let (after, _) = edited("p\r\nq\nr\n", "p\nq\n", "one\ntwo\nthree\n").await;
    assert_eq!(after, "one\r\ntwo\nthree\nr\n");
    // Fewer new lines: the endings of the first replaced lines.
    let (after, _) = edited("p\r\nq\nr\n", "p\nq\n", "pq\n").await;
    assert_eq!(after, "pq\r\nr\n");
}

#[test]
fn span_endings_list_each_line_break_of_the_span() {
    use crate::infrastructure::tools::filesystem::edit_bytes::{LineEnding, with_span_endings};
    assert_eq!(
        with_span_endings("a\nb\nc", "x\r\ny\n", LineEnding::Lf),
        "a\r\nb\nc"
    );
    // Beyond the span's own line breaks: the ending given.
    assert_eq!(
        with_span_endings("a\nb\nc", "x\r\ny", LineEnding::Crlf),
        "a\r\nb\r\nc"
    );
    assert_eq!(
        with_span_endings("a\nb\nc", "x\r\ny", LineEnding::Lf),
        "a\r\nb\nc"
    );
    // A lone \r in the span is no line ending of its own.
    assert_eq!(
        with_span_endings("a\nb", "x\ry", LineEnding::Crlf),
        "a\r\nb"
    );
    assert_eq!(with_span_endings("ab", "x\r\n", LineEnding::Lf), "ab");
}
