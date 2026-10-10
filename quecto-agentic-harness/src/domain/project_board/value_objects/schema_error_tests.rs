use super::*;

#[test]
fn a_line_allows_letters_marks_numbers_punctuation_symbols_and_spaces() {
    for good in [
        "Board store",
        "e\u{301}té",
        "№ 42 → ✓",
        "a\u{a0}b",
        "-",
        "日本語",
    ] {
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
fn markdown_keeps_newlines_but_not_carriage_returns_or_tabs() {
    assert_eq!(markdown("description", "", 10), Ok(()));
    assert_eq!(markdown("description", "# A\n\n- b", 10), Ok(()));
    for bad in ["a\r\nb", "a\tb", "\u{202e}"] {
        assert!(markdown("description", bad, 10).is_err(), "{bad:?}");
    }
}

#[test]
fn an_error_names_its_full_path() {
    let error = SchemaError::new("id", "bad").at("depends_on/3");
    assert_eq!(error.to_string(), "depends_on/3: bad");
}
