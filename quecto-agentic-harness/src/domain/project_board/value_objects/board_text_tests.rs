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
        ("\n\n  \nabc\n \n", "abc"),
        ("", ""),
        (" \n\t\n", ""),
    ] {
        assert_eq!(settled(text, raw), kept, "{raw:?}");
    }
}

#[test]
fn trailing_whitespace_goes_from_every_line() {
    assert_eq!(settled(text, "a  \nb\t\nc\u{a0}"), "a\nb\nc");
}

#[test]
fn spaces_a_reader_shows_as_one_become_one_in_prose() {
    for (raw, kept) in [
        ("a\u{a0}b", "a b"),
        ("a\u{2003}b\u{3000}c", "a b c"),
        ("a\tb", "a b"),
        // Inner runs are kept: the normaliser never collapses spaces.
        ("a  `x  y`  b", "a  `x  y`  b"),
    ] {
        assert_eq!(settled(text, raw), kept, "{raw:?}");
    }
}

#[test]
fn a_lone_presentation_selector_goes_and_an_emojis_stays() {
    for (raw, kept) in [
        ("A\u{fe0f}", "A"),
        ("a\u{fe0e}b", "ab"),
        ("\u{2764}\u{fe0f}", "\u{2764}\u{fe0f}"),
        ("1\u{fe0f}\u{20e3}", "1\u{fe0f}\u{20e3}"),
        ("\u{1f44d}\u{fe0f}", "\u{1f44d}\u{fe0f}"),
    ] {
        assert_eq!(settled(text, raw), kept, "{raw:?}");
    }
}

#[test]
fn text_is_made_nfc() {
    assert_eq!(settled(text, "e\u{301}t\u{e9}"), "\u{e9}t\u{e9}");
    assert_eq!(settled(text, "\u{212b}"), "\u{c5}");
}

#[test]
fn blank_line_runs_outside_code_become_one() {
    assert_eq!(settled(text, "a\n\n\n\nb"), "a\n\nb");
}

/// Up to three spaces before a block's first line show as nothing; four
/// start indented code, and a line inside a block keeps its spaces, so
/// those are left for the codec to refuse rather than change meaning.
#[test]
fn spaces_before_a_blocks_first_line_go_up_to_three() {
    for (raw, kept) in [
        ("   a", "a"),
        ("a\n\n  - b", "a\n\n- b"),
        ("    a", "    a"),
        ("- a\n  b", "- a\n  b"),
    ] {
        assert_eq!(settled(text, raw), kept, "{raw:?}");
    }
}

/// Inside a fence only what shows as nothing changes: trailing
/// whitespace, line ends and NFC. Tabs, no-break spaces, selectors and
/// blank lines are left as written.
#[test]
fn fenced_code_keeps_what_it_shows() {
    let raw = "```go\r\nfunc a() {\r\n\tx :=\u{a0}1  \r\n\r\n\r\n\ty\u{fe0f}\r\n}\r\n```\r\n\r\nafter\u{a0}it";
    let kept = "```go\nfunc a() {\n\tx :=\u{a0}1\n\n\n\ty\u{fe0f}\n}\n```\n\nafter it";
    assert_eq!(settled(text, raw), kept);
}

#[test]
fn a_line_is_trimmed_at_both_edges_and_fixed_inside() {
    for (raw, kept) in [
        ("  Title\u{a0}", "Title"),
        ("\u{a0}Fix\tthe\u{2003}build ", "Fix the build"),
        ("Ship it\u{fe0f}", "Ship it"),
        ("Cafe\u{301}", "Caf\u{e9}"),
    ] {
        assert_eq!(settled(line, raw), kept, "{raw:?}");
    }
}
