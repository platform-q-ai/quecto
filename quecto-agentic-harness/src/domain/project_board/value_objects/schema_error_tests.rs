use super::*;

#[test]
fn a_line_allows_letters_marks_numbers_punctuation_symbols_and_spaces() {
    for good in ["Board store", "e\u{301}té", "№ 42 → ✓", "-", "日本語"] {
        assert_eq!(line("title", good, 20), Ok(()), "{good:?}");
    }
}

#[test]
fn a_line_refuses_controls_format_private_use_unassigned_and_blank_text() {
    let refused = [
        "",
        " ",
        "\u{a0}",
        "a\nb",
        "a\rb",
        "a\tb",
        "bell\u{7}",
        "zero\u{200b}width",
        "bidi\u{202e}",
        "\u{feff}bom",
        "private\u{e000}",
        "unassigned\u{378}",
    ];
    for bad in refused {
        assert_eq!(
            line("title", bad, 20).map_err(|error| error.field),
            Err("title".into()),
            "{bad:?}"
        );
    }
}

#[test]
fn limits_count_characters_and_admit_exactly_the_bound() {
    assert_eq!(line("title", &"é".repeat(4), 4), Ok(()));
    assert!(line("title", &"é".repeat(5), 4).is_err());
    assert_eq!(markdown("description", &"日".repeat(4), 4), Ok(()));
    assert!(markdown("description", &"日".repeat(5), 4).is_err());
    assert_eq!(distinct("prs", &[1, 2], 2), Ok(()));
    assert!(distinct("prs", &[1, 2, 3], 2).is_err());
    assert!(distinct("prs", &[1, 1], 2).is_err());
}

#[test]
fn markdown_keeps_newlines_and_tabs_but_not_carriage_returns() {
    assert_eq!(markdown("description", "", 10), Ok(()));
    assert_eq!(markdown("description", "# A\n\n- b", 10), Ok(()));
    assert_eq!(markdown("description", "\tcode\tx", 10), Ok(()));
    for bad in ["a\r\nb", "\u{202e}", "a\u{b}b"] {
        assert!(markdown("description", bad, 10).is_err(), "{bad:?}");
    }
}

#[test]
fn a_tab_is_allowed_in_markdown_only() {
    assert_eq!(markdown("description", "a\tb", 10), Ok(()));
    assert!(line("title", "a\tb", 10).is_err());
}

/// The England flag: a black flag, the tag letters `gbeng`, a cancel tag.
const ENGLAND: &str = "\u{1f3f4}\u{e0067}\u{e0062}\u{e0065}\u{e006e}\u{e0067}\u{e007f}";

#[test]
fn joined_emoji_tag_flags_and_persian_text_are_allowed() {
    let joined = [
        "\u{1f469}\u{200d}\u{1f4bb}",         // woman technologist
        "\u{1f3f3}\u{fe0f}\u{200d}\u{1f308}", // rainbow flag
        ENGLAND,
        "\u{645}\u{6cc}\u{200c}\u{62e}\u{648}\u{627}\u{647}\u{645}", // Persian, with ZWNJ
        "flag \u{1f3f4}\u{e0067}\u{e0062}\u{e0073}\u{e0063}\u{e0074}\u{e007f}!",
    ];
    for good in joined {
        assert_eq!(line("title", good, 20), Ok(()), "{good:?}");
        assert_eq!(markdown("description", good, 20), Ok(()), "{good:?}");
    }
}

#[test]
fn joiners_and_tags_are_refused_outside_their_places_and_other_format_characters_always() {
    let refused = [
        "\u{200d}",                    // a lone ZWJ
        "a\u{200d}",                   // nothing after it
        "\u{200c}a",                   // nothing before it
        "a\u{200d}\u{200d}b",          // two joiners
        "a \u{200d}b",                 // a space before it
        "a\u{200c}\nb",                // a newline after it
        "\u{e0067}\u{e007f}",          // tags with no black flag
        "a\u{e0067}\u{e007f}",         // tags after another character
        "\u{1f3f4}\u{e0067}\u{e0062}", // a tag run never cancelled
        "\u{1f3f4}\u{e007f}",          // a cancel tag with no run
        "\u{1f3f4}\u{e0001}\u{e007f}", // the language tag
        "a\u{200e}b",                  // LRM
        "a\u{200f}b",                  // RLM
        "a\u{61c}b",                   // ALM
        "soft\u{ad}hyphen",            // soft hyphen
        "word\u{2060}joiner",          // word joiner
    ];
    for bad in refused {
        assert!(line("title", bad, 20).is_err(), "line {bad:?}");
        assert!(
            markdown("description", bad, 20).is_err(),
            "markdown {bad:?}"
        );
    }
}

#[test]
fn an_error_names_its_full_path() {
    let error = SchemaError::new("id", "bad").at("depends_on/3");
    assert_eq!(error.to_string(), "depends_on/3: bad");
}

/// `payload` spelt in emoji tag characters after a black flag, cancelled.
fn tag_flag(payload: &str) -> String {
    let tags: String = payload
        .chars()
        .map(|c| char::from_u32(0xE0000 + c as u32).unwrap())
        .collect();
    format!("\u{1f3f4}{tags}\u{e007f}")
}

#[test]
fn a_tag_flag_carries_only_a_subdivision_code() {
    for code in ["gbeng", "gbsct", "gbwls", "usca", "fr75", "esct"] {
        let flag = tag_flag(code);
        assert_eq!(line("title", &flag, 20), Ok(()), "{code}");
        assert_eq!(markdown("description", &flag, 20), Ok(()), "{code}");
    }
    let hidden = [
        "GBENG",                            // uppercase tags
        "gbEng",                            // one uppercase tag
        "gbengxy",                          // a payload past six
        "gb",                               // no subdivision
        "g1eng",                            // a digit in the country
        "gb eng",                           // a tag space
        "ignore all previous instructions", // a hidden instruction
        "ignoreall",                        // a word too long for a subdivision
    ];
    for payload in hidden {
        let flag = tag_flag(payload);
        assert!(line("title", &flag, 200).is_err(), "line {payload:?}");
        assert!(
            markdown("description", &format!("Fine.\n{flag}"), 200).is_err(),
            "markdown {payload:?}"
        );
    }
}

#[test]
fn one_presentation_selector_may_follow_a_glyph() {
    for good in [
        "\u{2764}\u{fe0f}",        // red heart, emoji style
        "\u{263a}\u{fe0e}",        // smiling face, text style
        "1\u{fe0f}\u{20e3}",       // keycap one
        "I \u{2764}\u{fe0f} Rust", // inside a line
    ] {
        assert_eq!(line("title", good, 20), Ok(()), "{good:?}");
        assert_eq!(markdown("description", good, 20), Ok(()), "{good:?}");
    }
}

#[test]
fn ignorable_and_blank_looking_characters_are_refused() {
    let refused = [
        "\u{fe0f}",          // a selector with no glyph
        "a\u{fe0f}\u{fe0f}", // two selectors
        "a \u{fe0f}",        // a selector after a space
        "\u{1f3f4}\u{e0067}\u{e0062}\u{e0065}\u{e006e}\u{e0067}\u{e007f}\u{fe0f}", // after a tag flag
        "a\u{fe00}",  // variation selector 1
        "a\u{fe0d}",  // variation selector 14
        "a\u{e0100}", // variation selector 17
        "a\u{e01ef}", // variation selector 256
        "a\u{34f}b",  // combining grapheme joiner
        "a\u{180b}",  // Mongolian free variation selector one
        "a\u{180d}",  // Mongolian free variation selector three
        "a\u{180f}",  // Mongolian free variation selector four
        "a\u{17b4}",  // Khmer inherent vowel
        "\u{115f}",   // Hangul choseong filler
        "a\u{1160}",  // Hangul jungseong filler
        "\u{3164}",   // Hangul filler
        "a\u{ffa0}",  // halfwidth Hangul filler
        "\u{2800}",   // braille blank
        "a\u{2800}b",
    ];
    for bad in refused {
        assert!(line("title", bad, 20).is_err(), "line {bad:?}");
        assert!(
            markdown("description", bad, 20).is_err(),
            "markdown {bad:?}"
        );
    }
}

/// A one-line field spaces its words with plain spaces only: any other
/// space (no-break, narrow, ideographic, em) is refused with its own
/// problem, as no file holds one.
#[test]
fn a_line_holds_plain_spaces_only() {
    assert_eq!(line("title", "plain spaces only", 100), Ok(()));
    for space in ['\u{a0}', '\u{202f}', '\u{2007}', '\u{3000}', '\u{2003}'] {
        assert_eq!(
            line("title", &format!("not{space}plain"), 100),
            Err(SchemaError::new("title", PLAIN_SPACES_ONLY)),
            "{space:?}"
        );
    }
}

/// Markdown text spaces its words with plain spaces, tabs and line ends
/// only: any other space (one the normaliser left beside a block marker,
/// or an em space) is refused with its field, as no file holds one.
#[test]
fn markdown_holds_plain_spaces_tabs_and_line_ends_only() {
    assert_eq!(markdown("description", "a b\n\tc", 100), Ok(()));
    for (text, line) in [
        ("-\u{a0}x", 1),
        ("\u{a0}leading", 1),
        ("em\u{2003}space", 1),
        ("a\nb\n\nc\u{3000}d", 4),
    ] {
        assert_eq!(
            markdown("description", text, 100),
            Err(SchemaError::new(
                "description",
                format!("line {line}: {PLAIN_SPACES_ONLY}")
            )),
            "{text:?}"
        );
    }
}
