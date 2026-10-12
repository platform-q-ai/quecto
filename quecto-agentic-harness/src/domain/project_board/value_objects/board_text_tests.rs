use super::*;

/// Every case reads as itself once normalised again.
fn settled(normalise: fn(&str) -> String, raw: &str) -> String {
    let once = normalise(raw);
    assert_eq!(normalise(&once), once, "not idempotent for {raw:?}");
    once
}

#[test]
fn line_ends_become_one_lf_and_edge_blank_lines_go() {
    for (raw, kept) in [
        ("Why.\r\nHow.\r\n", "Why.\nHow."),
        ("a\rb", "a\nb"),
        ("abc\n", "abc"),
        ("\n\n  \n\t\nabc\n \n", "abc"),
        ("", ""),
        (" \n\t\n", ""),
    ] {
        assert_eq!(settled(text, raw), kept, "{raw:?}");
    }
}

/// A trailing space or tab goes only where no hard break can form: a
/// single space, or before a blank line or the end.
#[test]
fn trailing_spaces_go_only_where_no_hard_break_forms() {
    for (raw, kept) in [
        ("a \nb", "a\nb"),
        ("a  \n\nb", "a\n\nb"),
        ("a\t\n\nb", "a\n\nb"),
        ("end  ", "end"),
        ("end \t ", "end"),
        // Two spaces before a line: a hard break, kept for the codec.
        ("line  \nnext", "line  \nnext"),
        ("line\t\nnext", "line\t\nnext"),
    ] {
        assert_eq!(settled(text, raw), kept, "{raw:?}");
    }
}

/// Inside a fence (```, ~~~, or longer) every line is kept as written.
#[test]
fn fenced_code_is_kept_as_written() {
    for raw in [
        "```\ncode  \n\nmore \n```",
        "~~~\ncode \n~~~",
        "````\n```\ncode \n````",
        "```\na\u{a0}\n```",
        "  ```\n  code \n  ```",
    ] {
        assert_eq!(settled(text, raw), raw, "{raw:?}");
    }
    // After a fence closes, prose is normalised again.
    assert_eq!(
        settled(text, "~~~\nx \n~~~\nafter \nnext"),
        "~~~\nx \n~~~\nafter\nnext"
    );
}

/// What the review found a wider normaliser changed in what GitHub shows
/// is left as written, for the codec to store or refuse.
#[test]
fn what_a_reader_shows_differently_is_left_as_written() {
    for raw in [
        "\tcode",
        "- a\n\n  b",
        "a\n\u{a0}\nb",
        "`a\tb` `a\u{a0}b` `a\u{3000}b` `x\u{fe0f}`",
        "line  \nnext",
        "\u{a0}- a",
        "A\u{fe0f} and a\tb and c\u{a0}d",
        "a\n\n\n\nb",
        "   a",
        "    a \nb",
    ] {
        assert_eq!(settled(text, raw), raw, "{raw:?}");
    }
}

#[test]
fn a_line_is_trimmed_of_spaces_and_tabs_at_both_edges() {
    for (raw, kept) in [
        ("  Title \t", "Title"),
        ("\tFix the build ", "Fix the build"),
        // Only spaces and tabs: a no-break space or a selector is kept.
        ("\u{a0}Ship it\u{fe0f}", "\u{a0}Ship it\u{fe0f}"),
    ] {
        assert_eq!(settled(line, raw), kept, "{raw:?}");
    }
}

#[test]
fn text_is_entered_only_once_normalised() {
    assert_eq!(entered_text("description", "Why.\nHow."), Ok(()));
    assert_eq!(entered_line("title", "Ship it"), Ok(()));
    let refused = entered_text("description", "Why.\r\n").unwrap_err();
    assert_eq!(refused.field, "description");
    let refused = entered_line("title", "Ship it ").unwrap_err();
    assert_eq!(refused.field, "title");
}

/// No NFC is taken: text written decomposed, or with characters NFC
/// would turn into markup (`>` and U+0338, U+1FEF, U+037E, the Kelvin
/// sign), is left as written, for the codec's NFC rule to refuse with its
/// line.
#[test]
fn nfc_is_never_taken() {
    for raw in [
        "Cafe\u{301}",
        "\u{212b}",
        ">\u{338} a",
        "\u{1fef}x\u{1fef}",
        "&lt\u{37e}b",
        "<\u{212a}bd>",
        "[cafe\u{301}]",
    ] {
        assert_eq!(settled(text, raw), raw, "{raw:?}");
        assert_eq!(settled(line, raw), raw, "{raw:?}");
    }
}

/// From a line that may start an HTML block on, the text is kept as
/// written: an HTML block keeps its lines' spaces.
#[test]
fn from_an_html_block_on_the_text_is_kept_as_written() {
    for raw in [
        "<!-- a \nb -->",
        "<div>\nx \ny\n</div>",
        "  <pre>\na \n</pre>",
        "a\n\n<p>b \nc",
    ] {
        assert_eq!(settled(text, raw), raw, "{raw:?}");
    }
    // Before it, prose is normalised.
    assert_eq!(
        settled(text, "a \nb\n\n<div>\nx \ny"),
        "a\nb\n\n<div>\nx \ny"
    );
}

/// A link title may run on across lines, showing a line's trailing space:
/// a paragraph with a title's start after `](` or `]:` is kept as written.
#[test]
fn a_paragraph_with_a_link_title_is_kept_as_written() {
    for raw in [
        "[a](/u \"t \nx\")",
        "[a](/u 't \nx')",
        "[a](/u (t \nx))",
        "[a]: /u \"t \nx\"",
    ] {
        assert_eq!(settled(text, raw), raw, "{raw:?}");
    }
}

/// A trailing space goes only after a safe character, and only in a
/// paragraph with no backtick or dollar so far (a code span or math may
/// run on across its lines).
#[test]
fn a_trailing_space_after_a_backslash_or_in_code_or_math_is_kept() {
    for raw in ["a\\ \nb", "`a \nb`", "x `y` \nz", "$x \ny$", "\\`a \nb"] {
        assert_eq!(settled(text, raw), raw, "{raw:?}");
    }
    // A blank line starts a new paragraph: its own lines are free again.
    assert_eq!(settled(text, "`x`\n\na \nb"), "`x`\n\na\nb");
}

/// Round 4: a link or reference anywhere in a paragraph (its title may
/// start on the next line, its label span lines), inline HTML (a tag, a
/// comment, a processing instruction or CDATA may span lines), and a
/// footnote definition (a container) keep what follows as written.
#[test]
fn links_inline_html_and_footnotes_are_kept_as_written() {
    for raw in [
        "[l]: /u \n\"t \nx\"\n\n[l]",
        "[a](/u \n\"t \nx\")",
        "[a](\n/u \"t \nx\")",
        "[a \nb](/u)",
        "a <span title=\"x \ny\">b</span>",
        "a <!-- x \ny -->",
        "a <?x y \nz ?>",
        "a <![CDATA[ x \ny ]]>",
        "[^1]: <pre> a \n\n[^1]",
        "[^1]: x\n\n    ```\n    code \n",
        "- [^n]: a \n  b",
    ] {
        assert_eq!(settled(text, raw), raw, "{raw:?}");
    }
}
