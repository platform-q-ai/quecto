use super::*;

#[test]
fn words_wrap_whole_and_only_an_overlong_word_is_broken() {
    assert_eq!(
        wrap_words("explicit association is not available yet", 12),
        ["explicit", "association", "is not", "available", "yet"]
    );
    assert_eq!(wrap_words("abcdefghij kl", 4), ["abcd", "efgh", "ij", "kl"]);
    assert_eq!(wrap_words("   ", 10), Vec::<String>::new());
    assert_eq!(
        wrap_words("a b", 0),
        ["a", "b"],
        "a zero width still progresses"
    );
    for line in wrap_words("the quick brown fox jumps over the lazy dog", 9) {
        assert!(visible_width(&line) <= 9, "{line:?}");
    }
}

#[test]
fn a_bounded_wrap_marks_its_cut_and_never_exceeds_the_width() {
    let text = "one two three four five six seven eight nine ten";
    assert_eq!(wrap_bounded(text, 10, 9), wrap_words(text, 10));
    let cut = wrap_bounded(text, 10, 2);
    assert_eq!(cut.len(), 2);
    assert!(cut[1].ends_with('…'), "{cut:?}");
    assert!(cut.iter().all(|line| visible_width(line) <= 10), "{cut:?}");
    assert_eq!(wrap_bounded(text, 10, 0), Vec::<String>::new());
    let narrow = wrap_bounded("abcdef ghijkl", 1, 1);
    assert_eq!(narrow.len(), 1);
}

#[test]
fn a_path_keeps_both_ends_and_fits_its_lines() {
    assert_eq!(path_lines("/work/other", 40, 2), ["/work/other"]);
    assert_eq!(path_lines("/abcdefgh", 5, 2), ["/abcd", "efgh"]);
    let long = format!("/home/user/{}/projects/quecto", "deep/".repeat(40));
    let lines = path_lines(&long, 30, 2);
    assert_eq!(lines.len(), 2);
    assert!(
        lines.iter().all(|line| visible_width(line) <= 30),
        "{lines:?}"
    );
    let shown = lines.concat();
    assert!(shown.starts_with("/home/user/"), "{shown}");
    assert!(shown.ends_with("/projects/quecto"), "{shown}");
    assert_eq!(shown.matches('…').count(), 1, "{shown}");
    assert_eq!(path_lines("", 10, 2), Vec::<String>::new());
    // Wide characters count by column, and a zero width still progresses.
    let wide = path_lines("/日本語/フォルダ", 6, 3);
    assert!(wide.iter().all(|line| visible_width(line) <= 6), "{wide:?}");
    assert!(!path_lines("/ab", 0, 0).is_empty());
}

/// Review R2-T2: a bound keeps BOTH ends — never a head-only cut.
#[test]
fn a_bound_drops_the_middle_and_keeps_both_ends() {
    assert_eq!(bounded_ends("short", 512), "short");
    assert_eq!(bounded_ends("abcdefghij", 10), "abcdefghij");
    assert_eq!(bounded_ends("abcdefghijk", 10), "abcd…ghijk");
    let long = format!("/home/u/{}project-ONE", "deep/".repeat(120));
    let bounded = bounded_ends(&long, 512);
    assert_eq!(bounded.chars().count(), 512);
    assert!(bounded.starts_with("/home/u/deep/") && bounded.ends_with("/project-ONE"));
    assert_eq!(bounded_ends("äöüäöüäöü", 4), "ä…öü");
    assert_eq!(bounded_ends("abc", 0), "…");
}
