use super::*;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

fn normalize_for_fuzzy_match(s: &str) -> String {
    fuzzy_normalise(s).text
}

fn unique(content: &str, old: &str) -> Range<usize> {
    match locate(content, old) {
        Ok(Location::Unique(range)) => range,
        other => panic!("expected one match of {old:?} in {content:?}, got {other:?}"),
    }
}

// --- fuzzy normaliser ---

#[test]
fn test_fuzzy_normalise_smart_single_quotes() {
    // U+2018 U+2019 U+201A U+201B → '
    for ch in ['\u{2018}', '\u{2019}', '\u{201A}', '\u{201B}'] {
        let input = format!("it{ch}s");
        let result = normalize_for_fuzzy_match(&input);
        assert_eq!(result, "it's", "char U+{:04X} should become '", ch as u32);
    }
}

#[test]
fn test_fuzzy_normalise_smart_double_quotes() {
    // U+201C U+201D U+201E U+201F → "
    for ch in ['\u{201C}', '\u{201D}', '\u{201E}', '\u{201F}'] {
        let input = format!("{ch}hello{ch}");
        let result = normalize_for_fuzzy_match(&input);
        assert_eq!(
            result, "\"hello\"",
            "char U+{:04X} should become \"",
            ch as u32
        );
    }
}

#[test]
fn test_fuzzy_normalise_unicode_dashes() {
    // U+2010–U+2015, U+2212 → -
    for ch in [
        '\u{2010}', '\u{2011}', '\u{2012}', '\u{2013}', '\u{2014}', '\u{2015}', '\u{2212}',
    ] {
        let input = format!("a{ch}b");
        let result = normalize_for_fuzzy_match(&input);
        assert_eq!(result, "a-b", "char U+{:04X} should become -", ch as u32);
    }
}

#[test]
fn test_fuzzy_normalise_trailing_whitespace_per_line() {
    let input = "hello   \nworld  \n";
    let result = normalize_for_fuzzy_match(input);
    assert_eq!(result, "hello\nworld\n");
}

#[test]
fn test_fuzzy_normalise_special_spaces() {
    // NBSP and ideographic space → regular space
    let input = "a\u{00A0}b\u{3000}c";
    let result = normalize_for_fuzzy_match(input);
    assert_eq!(result, "a b c");
}

#[test]
fn test_fuzzy_normalise_keeps_every_newline() {
    assert_eq!(normalize_for_fuzzy_match("a  \n\n b \n"), "a\n\n b\n");
    assert_eq!(normalize_for_fuzzy_match("a\n"), "a\n");
    assert_eq!(normalize_for_fuzzy_match(""), "");
}

// --- the offset map ---

#[test]
fn every_normalised_byte_maps_to_the_character_that_made_it() {
    let text = "\u{2019}a  \n\u{00A0}\u{2014}\n";
    let fuzzy = fuzzy_normalise(text);
    assert_eq!(fuzzy.text, "'a\n -\n");
    assert_eq!(fuzzy.source, vec![0, 3, 6, 7, 9, 12]);
}

#[test]
fn a_multi_byte_character_kept_as_is_maps_every_byte_to_its_start() {
    let fuzzy = fuzzy_normalise("x\u{754C}");
    assert_eq!(fuzzy.text, "x\u{754C}");
    assert_eq!(fuzzy.source, vec![0, 1, 1, 1]);
}

#[test]
fn original_range_refuses_empty_out_of_bounds_and_mid_character_ranges() {
    let text = "a\u{754C}b";
    let fuzzy = fuzzy_normalise(text);
    assert_eq!(fuzzy.original_range(text, 1..1), None);
    assert_eq!(fuzzy.original_range(text, 0..9), None);
    assert_eq!(fuzzy.original_range(text, 2..4), None);
    assert_eq!(fuzzy.original_range(text, 0..2), None);
    assert_eq!(fuzzy.original_range(text, 1..4), Some(1..4));
    assert_eq!(fuzzy.original_range(text, 0..5), Some(0..5));
}

// --- locate ---

#[test]
fn an_exact_match_wins_over_a_fuzzy_one() {
    assert_eq!(unique("it\u{2019}s it's", "it's"), 7..11);
}

#[test]
fn exact_and_fuzzy_ambiguity_are_both_refused() {
    assert_eq!(locate("ab ab", "ab"), Ok(Location::Ambiguous(2)));
    assert_eq!(
        locate("\u{2018}x\u{2019} 'x'", "\u{2019}x'"),
        Ok(Location::Ambiguous(2))
    );
}

#[test]
fn overlapping_matches_are_ambiguous() {
    assert_eq!(locate("aaa", "aa"), Ok(Location::Ambiguous(2)));
    assert_eq!(
        locate("\u{2019}-'-\u{2018}", "'-'"),
        Ok(Location::Ambiguous(2))
    );
    assert_eq!(unique("\u{754C}\u{754C}a", "\u{754C}a"), 3..7);
}

#[test]
fn a_missing_or_whitespace_only_old_text_is_not_found() {
    assert_eq!(locate("abc", "xyz"), Ok(Location::NotFound));
    assert_eq!(locate("a  \nb", "   "), Ok(Location::NotFound));
}

#[test]
fn dropped_whitespace_inside_the_match_is_replaced_with_it() {
    assert_eq!(unique("foo  \nbar\nbaz\n", "foo\nbar"), 0..9);
}

#[test]
fn dropped_whitespace_at_either_edge_of_the_match_is_kept() {
    let content = "pad  \nfoo  \nbar\n";
    assert_eq!(unique(content, "\nfoo"), 5..9);
    assert_eq!(unique(content, "foo\t"), 6..9);
}

#[test]
fn a_match_ending_on_a_newline_ends_after_that_newline() {
    assert_eq!(unique("a\u{2013}b  \nc\n", "a-b\n"), 0..8);
}

#[test]
fn a_curly_quote_is_replaced_whole() {
    assert_eq!(unique("\u{2019}\u{2019}ab\u{2019}\n", "b'"), 7..11);
    assert_eq!(unique("it\u{2019}s here\n", "it's here"), 0..11);
}

#[test]
fn a_mapped_range_that_does_not_normalise_to_the_needle_is_refused() {
    assert_eq!(prove_fuzzy_range("a b", &(0..3), "ab"), Err(Unmappable));
    assert_eq!(
        prove_fuzzy_range("a\u{754C}", &(0..2), "a"),
        Err(Unmappable)
    );
    assert_eq!(prove_fuzzy_range("\u{2019}b", &(0..4), "'b"), Ok(()));
}

// --- property: fuzzy edits never touch bytes outside the matched text ---

const ALPHABET: &[&str] = &[
    "a",
    "b",
    "x",
    " ",
    " ",
    "\t",
    "\n",
    "\n",
    "'",
    "\"",
    "-",
    "\u{2019}",
    "\u{2018}",
    "\u{201C}",
    "\u{201D}",
    "\u{2013}",
    "\u{2014}",
    "\u{00A0}",
    "\u{3000}",
    "\u{754C}",
    "\u{E9}",
    "\u{1F680}",
];

fn random_text(rng: &mut StdRng, max_chars: usize) -> String {
    let len = rng.gen_range(0..=max_chars);
    (0..len)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())])
        .collect()
}

/// Rewrite `region` the way a model might retype it: swap quotes, dashes and
/// spaces for look-alikes, and add or drop trailing spaces before newlines.
fn retype(rng: &mut StdRng, region: &str) -> String {
    let mut out = String::new();
    for c in region.chars() {
        if c == '\n' && rng.gen_bool(0.3) {
            let trimmed = out.trim_end_matches([' ', '\t']).len();
            out.truncate(trimmed);
            out.push_str(&" ".repeat(rng.gen_range(0..3)));
        }
        let swap = rng.gen_bool(0.5);
        let retyped = match c {
            '\u{2019}' | '\u{2018}' if swap => '\'',
            '\'' if swap => '\u{2019}',
            '\u{201C}' | '\u{201D}' if swap => '"',
            '"' if swap => '\u{201C}',
            '\u{2013}' | '\u{2014}' if swap => '-',
            '-' if swap => '\u{2014}',
            '\u{00A0}' if swap => ' ',
            other => other,
        };
        out.push(retyped);
    }
    out
}

fn char_boundaries(text: &str) -> Vec<usize> {
    text.char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(text.len()))
        .collect()
}

#[test]
fn fuzzy_edits_only_change_the_matched_text_and_never_panic() {
    let mut rng = StdRng::seed_from_u64(0x2191);
    let mut unique_fuzzy = 0usize;
    for _ in 0..20_000 {
        let content = random_text(&mut rng, 40);
        let bounds = char_boundaries(&content);
        let i = bounds[rng.gen_range(0..bounds.len())];
        let j = bounds[rng.gen_range(0..bounds.len())];
        let (i, j) = (i.min(j), i.max(j));
        let old = retype(&mut rng, &content[i..j]);
        let is_exact = content.contains(old.as_str()) && !old.is_empty();

        let location = locate(&content, &old);
        let Ok(location) = location else {
            panic!("unmappable: {content:?} / {old:?}");
        };
        let Location::Unique(range) = location else {
            let fuzzy_old_is_empty = fuzzy_normalise(&old).text.is_empty();
            let is_refusal = matches!(location, Location::Ambiguous(_))
                || (location == Location::NotFound && fuzzy_old_is_empty);
            assert!(is_refusal, "{location:?} for {old:?} in {content:?}");
            continue;
        };
        let replaced = content
            .get(range.clone())
            .expect("range on char boundaries");
        let edited = splice_for_test(&content, &range, "<NEW>");
        assert_eq!(edited.get(..range.start), content.get(..range.start));
        assert!(edited.ends_with(&content[range.end..]));
        if is_exact {
            assert_eq!(replaced, old, "{content:?}");
            continue;
        }
        unique_fuzzy += 1;
        assert!(
            i <= range.start && range.end <= j,
            "fuzzy match {range:?} outside intended {i}..{j}: {content:?} / {old:?}"
        );
        assert_eq!(fuzzy_normalise(replaced).text, fuzzy_normalise(&old).text);
    }
    assert!(
        unique_fuzzy > 1_000,
        "too few fuzzy edits exercised: {unique_fuzzy}"
    );
}

fn splice_for_test(content: &str, range: &Range<usize>, new: &str) -> String {
    format!(
        "{}{}{}",
        &content[..range.start],
        new,
        &content[range.end..]
    )
}
