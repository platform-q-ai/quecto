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
        "A\u{fe0f} and a\tb",
        "a\n\n\n\nb",
        "   a",
        "    a \nb",
    ] {
        assert_eq!(settled(text, raw), raw, "{raw:?}");
    }
}

/// A line's line ends become one LF (a chat line or paragraph separator,
/// a next line and a vertical tab among them), the spaces chat writes
/// between words plain spaces, and its edge spaces and tabs go. A
/// selector, or another space (an em space), is kept for the domain to
/// refuse with its field.
#[test]
fn a_line_is_trimmed_of_spaces_and_tabs_at_both_edges() {
    for (raw, kept) in [
        ("  Title \t", "Title"),
        ("\tFix the build ", "Fix the build"),
        ("\u{a0}Ship it\u{fe0f}", "Ship it\u{fe0f}"),
        ("Bonjour\u{202f}: on commence ?", "Bonjour : on commence ?"),
        ("\u{2007}12\u{a0}items\u{3000}", "12 items"),
        ("em\u{2003}space", "em\u{2003}space"),
        ("one\u{2028}two", "one\ntwo"),
        // A tab inside is a plain space; a line end at an edge goes.
        ("a\tb", "a b"),
        ("Title\n", "Title"),
        ("Title\u{2028}", "Title"),
        ("\u{2029}\t Title \r\n", "Title"),
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
        "a <span title=\"x \ny\">b</span>",
        "a <!-- x \ny -->",
        "a <?x y \nz ?>",
        "a <![CDATA[ x \ny ]]>",
        "[^1]: <pre> a \n\n[^1]",
        "[^1]: x\n\n    ```\n    code ",
        "- [^n]: a \n  b",
    ] {
        assert_eq!(settled(text, raw), raw, "{raw:?}");
    }
    // Code in a footnote running to the end keeps its spaces; only the
    // final line end, which no reader shows, goes.
    assert_eq!(
        settled(text, "[^1]: x\n\n    ```\n    code \n"),
        "[^1]: x\n\n    ```\n    code "
    );
    // A label that only closes on a later line: its space before the
    // line end shows as nothing either way, and goes.
    assert_eq!(settled(text, "[a \nb](/u)"), "[a\nb](/u)");
}

/// A message as it comes in (#2481 D3): its line ends one LF, the spaces
/// chat carries (no-break, narrow no-break, figure and ideographic) plain
/// spaces, each line trimmed of spaces and tabs at its edges (a table cell
/// shows none), and blank lines dropped (a message is its non-empty
/// lines). Anything else (a selector, another space) is kept, for the
/// domain or the codec to refuse with its field or line.
#[test]
fn a_message_is_its_lines_trimmed_with_blank_lines_dropped() {
    for (raw, kept) in [
        ("Lease?", "Lease?"),
        ("  Lease? \t", "Lease?"),
        (
            "\r\n Yes,\r\n\r\n\twith a TTL. \r\n \n",
            "Yes,\nwith a TTL.",
        ),
        ("one\rtwo\n\n\nthree", "one\ntwo\nthree"),
        ("  `a  b`  ", "`a  b`"),
        ("\u{a0}ok\u{fe0f}", "ok\u{fe0f}"),
        ("Pocket:\u{a0}yes", "Pocket: yes"),
        ("Bonjour\u{202f}: oui", "Bonjour : oui"),
        ("\u{2007}12\u{2007}", "12"),
        ("はい\u{3000}そうです", "はい そうです"),
        ("\u{a0}\u{3000}\nnext", "next"),
        ("em\u{2003}space", "em\u{2003}space"),
        ("one\u{2028}two", "one\ntwo"),
        ("one\u{2029}two", "one\ntwo"),
        ("one\u{85}two", "one\ntwo"),
        ("one\u{b}two", "one\ntwo"),
        ("a\ttab inside", "a tab inside"),
        (" \t\r\n\n", ""),
    ] {
        assert_eq!(settled(message, raw), kept, "{raw:?}");
    }
}

#[test]
fn a_message_is_entered_only_once_normalised() {
    assert_eq!(entered_message("batch/0/text", "Yes,\nwith a TTL."), Ok(()));
    for raw in [
        "Yes, ",
        "Yes,\n\nno",
        " Yes",
        "Yes,\r\nno",
        "Yes,\n",
        "Yes,\u{a0}no",
    ] {
        assert_eq!(
            entered_message("batch/0/text", raw),
            Err(SchemaError::new(
                "batch/0/text",
                "has spacing the board removes: use one LF between lines, no blank lines, and \
                 no spaces or tabs at the start or end of a line"
            )),
            "{raw:?}"
        );
    }
}

/// A one-line field never normalised is refused with the owner-approved
/// problem for a line.
#[test]
fn a_line_never_normalised_says_what_a_line_is() {
    assert_eq!(
        entered_line("title", " Ship it"),
        Err(SchemaError::new(
            "title",
            "must be one line, use plain spaces only, and have no spaces or tabs at its \
             start or end"
        ))
    );
}

/// In markdown text a space chat writes between words (no-break, narrow
/// no-break, figure, ideographic: French keyboards put them before `!`,
/// `?`, `:` and inside guillemets) becomes a plain space where nothing can
/// change: in a prose line, past its first word, between characters a
/// space beside them never makes markup of. Everywhere else it is left as
/// written, for the domain to refuse.
#[test]
fn a_chat_space_between_words_is_made_plain_in_text() {
    for (raw, kept) in [
        (
            "Bonjour\u{202f}! On commence\u{a0}?",
            "Bonjour ! On commence ?",
        ),
        (
            "Il dit\u{a0}: \u{ab}\u{a0}oui\u{a0}\u{bb}\nPrix\u{a0}: 10\u{a0}000\u{a0}\u{20ac}",
            "Il dit : \u{ab} oui \u{bb}\nPrix : 10 000 \u{20ac}",
        ),
        (
            "\u{306f}\u{3044}\u{3000}\u{305d}\u{3046}",
            "\u{306f}\u{3044} \u{305d}\u{3046}",
        ),
        ("- item\u{a0}one", "- item one"),
        ("1. item\u{202f}!", "1. item !"),
        ("12\u{a0}items", "12 items"),
        // A bracket keeps the rest of its paragraph only: a blank line, or
        // a fence's close, ends it.
        ("[a] b\n\nc\u{a0}!", "[a] b\n\nc !"),
        ("[a\n```\nx\n```\nc\u{a0}!", "[a\n```\nx\n```\nc !"),
        // One neighbour a letter or digit is enough.
        ("\u{ab}\u{a0}oui\u{a0}\u{bb}", "\u{ab} oui \u{bb}"),
    ] {
        assert_eq!(settled(text, raw), kept, "{raw:?}");
    }
    for raw in [
        // A block marker or indentation it could make.
        "-\u{a0}x",
        "1.\u{a0}x",
        "\u{a0}leading",
        "#\u{a0}x",
        "> \u{a0}x",
        // Markup a plain space beside it would make.
        "a\u{a0}#",
        "- [\u{a0}] x",
        "a\u{a0}\u{a0}b",
        "end\u{a0}",
        "a\u{a0}*b*",
        "\u{3e}1.\u{a0}x",
        "- [x]\u{a0}a",
        "*a*\u{a0}b",
        // Code, links, references, addresses and math are kept as written:
        // an autolink reads a chat space as part of the address, a
        // reference label folds it apart from a plain space.
        "`a\u{a0}b`",
        "[a\u{a0}b](https://example.com)",
        "[a\u{a0}b]",
        "[a\nb\u{a0}c\nd]",
        "See www.example.com\u{a0}now",
        "See\u{a0}www.example.com now",
        "SEE\u{a0}WWW.EXAMPLE.COM",
        "Go https://example.com\u{a0}now",
        "Mail a@b.co\u{a0}now",
        // An address a combining mark splits still reads as one.
        "x w\u{301}ww.a.co\u{a0}now",
        // Neither neighbour a letter or digit: a table's delimiter row.
        "| a |\n|:-:\u{a0}:-:|",
        "100%\u{a0}!",
        "```\na\u{a0}b\n```",
        "    a\u{a0}b",
        // A space no keyboard puts between words.
        "em\u{2003}space",
    ] {
        assert_eq!(settled(text, raw), raw, "{raw:?}");
    }
}
