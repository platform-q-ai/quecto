use super::*;

#[test]
fn file_lines_count_only_the_files_own_line_breaks() {
    let lines = FileLines::of("a\rb\nc\r\nd");
    // Normalised: "a\nb\nc\nd"; the first '\n' was a lone '\r'.
    assert_eq!(lines.line_of(0), 1);
    assert_eq!(lines.line_of(2), 1);
    assert_eq!(lines.line_of(3), 1);
    assert_eq!(lines.line_of(4), 2);
    assert_eq!(lines.line_of(6), 3);
}

#[test]
fn file_lines_skip_a_byte_order_mark() {
    let lines = FileLines::of("\u{FEFF}x\ny");
    assert_eq!(lines.line_of(1), 1);
    assert_eq!(lines.line_of(2), 2);
}

/// Offsets agree with the normalised text for any mix of line endings.
#[test]
fn file_lines_follow_the_normalised_text() {
    for raw in [
        "",
        "\r",
        "\r\r\n\n",
        "\u{754C}\r\u{1F680}\r\n",
        "\u{FEFF}\n\r",
    ] {
        let lines = FileLines::of(raw);
        let text = base_normalise(raw);
        let real = raw.matches('\n').count();
        assert_eq!(lines.line_of(text.len() + 1), real + 1, "{raw:?}");
    }
}

/// #2193 review 2: a failed write that may have truncated the file says so,
/// and how to get the text back; only an error on opening means nothing
/// was written.
#[test]
fn a_write_failure_after_opening_warns_that_text_may_be_lost() {
    let error = std::io::Error::other("disk full");
    let refusal = write_refusal("a b.txt", &error);
    assert!(refusal.is_error);
    assert_eq!(
        refusal.content,
        "writing a b.txt failed: disk full. The file may now be empty or cut short and its \
         earlier text lost; restore it (e.g. git checkout -- 'a b.txt') or rewrite it with write."
    );
    let denied = write_refusal(
        "f",
        &std::io::Error::from(std::io::ErrorKind::PermissionDenied),
    );
    assert!(
        denied.content.contains("nothing was written"),
        "{}",
        denied.content
    );
    let read_only = write_refusal(
        "f",
        &std::io::Error::from(std::io::ErrorKind::ReadOnlyFilesystem),
    );
    assert!(
        read_only.content.contains("nothing was written"),
        "{}",
        read_only.content
    );
}

/// #2193 review 2: more proven matches than are listed are said to exist.
#[test]
fn an_indentation_hint_with_more_matches_says_so() {
    let content = "\tfoo\n".repeat(5);
    let refusal = not_found(&content, &FileLines::of(&content), "  foo", "f");
    assert!(
        refusal
            .content
            .contains("it matches at lines 1, 2, 3 and more if indentation is ignored"),
        "{}",
        refusal.content
    );
}
