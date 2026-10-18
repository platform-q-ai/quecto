use super::*;

/// The hint as 1-based lines: where the match starts, the line whose
/// indentation differs, and the two indentations.
type Hint = (usize, usize, String, String);

fn hint(content: &str, old: &str) -> Option<Hint> {
    let line = |at: usize| content[..at].matches('\n').count() + 1;
    indent_mismatch(content, old).map(|near| {
        (
            line(near.start_at),
            line(near.differs_at),
            near.file_indent,
            near.old_indent,
        )
    })
}

fn mismatch(start: usize, line: usize, file_indent: &str, old_indent: &str) -> Option<Hint> {
    Some((start, line, file_indent.to_string(), old_indent.to_string()))
}

/// #2193 (B13): spaces in oldText, a tab in the file.
#[test]
fn a_line_indented_with_a_tab_is_found_for_spaces() {
    assert_eq!(
        hint("\tindented text\n", "    indented text"),
        mismatch(1, 1, "\t", "    ")
    );
}

#[test]
fn the_first_line_whose_indentation_differs_is_named() {
    let content = "fn a() {\n\tlet x = 1;\n\tlet y = 2;\n}\n";
    assert_eq!(
        hint(content, "fn a() {\n    let x = 1;\n    let y = 2;"),
        mismatch(1, 2, "\t", "    ")
    );
}

/// oldText may start mid-line: its first line's indentation is not compared.
#[test]
fn an_unindented_first_line_may_start_mid_line() {
    assert_eq!(
        hint("x = foo(\n\t\tbar)\n", "foo(\n  bar)"),
        mismatch(1, 2, "\t\t", "  ")
    );
}

/// An indented first line stands for a whole line, not the end of one.
#[test]
fn an_indented_first_line_does_not_match_mid_line() {
    assert_eq!(hint("barfoo\n", "    foo"), None);
}

/// The hint is proven: with the file's indentation, oldText must match.
#[test]
fn a_difference_other_than_indentation_gets_no_hint() {
    assert_eq!(hint("xy\n", "x "), None);
    assert_eq!(hint("\txy\n", "  x "), None);
    assert_eq!(hint("\tfoo bar\n", "  foo  baz"), None);
    assert_eq!(hint("abc\n", "xyz"), None);
}

#[test]
fn matching_text_has_no_indentation_mismatch() {
    assert_eq!(hint("\tfoo\n", "\tfoo"), None);
}

#[test]
fn look_alike_characters_and_indentation_together_are_hinted() {
    assert_eq!(
        hint("\tsay \"hi\"\n", "    say \u{201C}hi\u{201D}"),
        mismatch(1, 1, "\t", "    ")
    );
}

#[test]
fn multibyte_text_is_hinted_on_its_line() {
    assert_eq!(
        hint("a\n\t\u{754C}\u{1F680}\n", "  \u{754C}\u{1F680}"),
        mismatch(2, 2, "\t", "  ")
    );
}

/// A mid-line candidate does not hide a later one on its own lines.
#[test]
fn a_mid_line_candidate_does_not_hide_a_later_one() {
    let content = "xfoo\n\tbaz\n\tfoo\n\tbaz\n";
    assert_eq!(hint(content, "  foo\n  baz"), mismatch(3, 3, "\t", "  "));
}

/// The proof is made on the candidate's own lines, so the line named is
/// the one that matches, not an earlier look-alike.
#[test]
fn the_line_named_is_the_one_the_proof_matched() {
    assert_eq!(
        hint("\tfoox\n\tfoo \n", "  foo "),
        mismatch(2, 2, "\t", "  ")
    );
}

/// Blank lines are not indentation: fuzzy matching already forgives them.
#[test]
fn a_blank_line_is_not_named_as_the_difference() {
    assert_eq!(
        hint("a\n\n\tb\n", "a\n   \n  b"),
        mismatch(1, 3, "\t", "  ")
    );
}

#[test]
fn indentation_is_described_in_words() {
    assert_eq!(describe_indent(""), "no indentation");
    assert_eq!(describe_indent("\t"), "1 tab");
    assert_eq!(describe_indent("    "), "4 spaces");
    assert_eq!(describe_indent(" "), "1 space");
    assert_eq!(describe_indent("\t\t  "), "2 tabs then 2 spaces");
    assert_eq!(
        describe_indent("\u{00A0}\u{00A0}"),
        "2 other whitespace characters"
    );
}

/// An unindented first line is not compared even when it starts a line
/// that the file indents: oldText may begin after the indentation.
#[test]
fn an_unindented_first_line_is_not_named_even_at_a_line_start() {
    assert_eq!(
        hint("\tfoo\n\tbar\n", "foo\n  bar"),
        mismatch(1, 2, "\t", "  ")
    );
}

/// A whitespace-only first line indents nothing, so it may start mid-line.
#[test]
fn a_whitespace_only_first_line_counts_as_unindented() {
    assert_eq!(
        hint("x  \n\tfoo\n", "  \n  foo"),
        mismatch(1, 2, "\t", "  ")
    );
}

/// The match's start and the differing line are named apart.
#[test]
fn the_match_start_and_the_differing_line_are_both_given() {
    let content = "fn a() {\n    keep();\n\tchange();\n}\n";
    assert_eq!(
        hint(content, "fn a() {\n    keep();\n    change();"),
        mismatch(1, 3, "\t", "    ")
    );
}

/// #2193 review 2: when the re-indented text matches in more than one
/// place, every place is listed, not just the first.
#[test]
fn every_proven_match_is_listed() {
    let near = indent_mismatch("\tfoo\n\tfoo\n", "  foo").unwrap();
    assert_eq!(
        (near.start_at, near.also_at.clone(), near.more),
        (0, vec![5], false)
    );
    let many = "\tfoo\n".repeat(5);
    let near = indent_mismatch(&many, "  foo").unwrap();
    assert_eq!(
        (near.start_at, near.also_at, near.more),
        (0, vec![5, 10], true)
    );
    let near = indent_mismatch("\tfoo\n", "  foo").unwrap();
    assert_eq!((near.also_at, near.more), (vec![], false));
}
