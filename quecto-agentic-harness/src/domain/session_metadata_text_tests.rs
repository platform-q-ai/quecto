use super::*;

fn finds(query: &str, title: &str) -> bool {
    let (query, title) = (visible_text(query), visible_text(title));
    query.split(' ').all(|term| title.contains(term))
}

/// R1-H3, the reviewer's cases: the natural spelling of a word finds its
/// title whatever the case either was typed in.
#[test]
fn a_case_fold_not_a_per_character_lower_casing_decides_a_match() {
    // Final sigma: the typed word ends in `ς`, the capitals have only `Σ`.
    assert!(finds("οδος", "ΟΔΟΣ plan"));
    assert!(finds("οδοσ", "ΟΔΟΣ plan"));
    assert!(finds("ΟΔΟΣ", "ένας οδος"));
    assert_eq!(visible_text("ΣΊΣΥΦΟΣ"), visible_text("σίσυφος"));
    // Dotted capital I lower-cases to `i` + U+0307; dotless `ı` is an `i`.
    assert!(finds("istanbul", "İstanbul notes"));
    assert!(finds("İSTANBUL", "istanbul notes"));
    assert!(finds("ısparta", "ISPARTA trip"));
    assert!(finds("isparta", "Isparta trip"));
    // Sharp s, both capitals of it.
    assert!(finds("straße", "STRASSE fix"));
    assert!(finds("strasse", "Straße fix"));
    assert!(finds("STRAẞE", "strasse fix"));
}

#[test]
fn the_fold_strips_only_the_dot_a_dotted_i_brings_never_another_combining_mark() {
    assert_eq!(visible_text("İ"), "i");
    // A dot above on any other letter is that letter's own mark.
    assert_eq!(visible_text("A\u{307}"), "a\u{307}");
    assert_eq!(visible_text("i\u{301}"), "i\u{301}");
    // No normalization: composed and decomposed spellings stay two texts.
    assert!(!finds("caf\u{e9}", "cafe\u{301} notes"));
    assert!(finds("cafe", "cafe\u{301} notes"));
}

#[test]
fn a_path_that_is_text_is_itself_and_a_byte_that_is_not_is_spelled() {
    use std::os::unix::ffi::OsStrExt;
    let path = |bytes: &'static [u8]| std::path::PathBuf::from(std::ffi::OsStr::from_bytes(bytes));
    assert_eq!(
        display_path(&path("/w/日本 語/é".as_bytes())),
        "/w/日本 語/é"
    );
    assert_eq!(display_path(&path(b"/w/caf\xe9/x")), "/w/caf\\xE9/x");
    assert_eq!(display_path(&path(b"\xff\xfe")), "\\xFF\\xFE");
    // A truncated multi-byte sequence: each stray byte, nothing swallowed.
    assert_eq!(display_path(&path(b"a\xe2\x80z")), "a\\xE2\\x80z");
    assert_eq!(display_path(std::path::Path::new("")), "");
}
