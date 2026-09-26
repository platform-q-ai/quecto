use super::*;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

fn normalize_for_fuzzy_match(s: &str) -> String {
    fuzzy_normalise(s).unwrap().text
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
    let fuzzy = fuzzy_normalise(text).unwrap();
    assert_eq!(fuzzy.text, "'a\n -\n");
    assert_eq!(fuzzy.source, vec![0, 3, 6, 7, 9, 12]);
}

#[test]
fn a_multi_byte_character_kept_as_is_maps_every_byte_to_its_start() {
    let fuzzy = fuzzy_normalise("x\u{754C}").unwrap();
    assert_eq!(fuzzy.text, "x\u{754C}");
    assert_eq!(fuzzy.source, vec![0, 1, 1, 1]);
}

#[test]
fn original_range_refuses_empty_out_of_bounds_and_mid_character_ranges() {
    let text = "a\u{754C}b";
    let fuzzy = fuzzy_normalise(text).unwrap();
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
fn file_whitespace_at_an_edge_is_kept_when_old_text_has_none_there() {
    let content = "pad  \nfoo  \nbar\n";
    assert_eq!(unique(content, "\nfoo"), 5..9);
    assert_eq!(unique("pad  \nfo\u{2019}  \n", "fo'"), 6..11);
}

#[test]
fn file_whitespace_at_an_edge_goes_with_the_match_when_old_text_has_some() {
    let content = "pad  \nfoo  \nbar\n";
    assert_eq!(unique(content, "foo\t"), 6..11);
    assert_eq!(unique(content, "\t\nfoo"), 3..9);
    assert_eq!(unique("a\u{2019}\u{00A0}\tb", "a' "), 0..7);
}

#[test]
fn old_text_ending_in_whitespace_needs_file_whitespace_or_a_line_end() {
    assert_eq!(
        locate("it\u{2019}s foobar", "it's foo "),
        Ok(Location::NotFound)
    );
    assert_eq!(unique("it\u{2019}s foo", "it's foo "), 0..10);
    assert_eq!(unique("it\u{2019}s\nfoo", "it's\t"), 0..6);
}

#[test]
fn edges_are_read_from_old_text() {
    let edges = |old: &str| {
        let e = Edges::of(old);
        (e.before, e.after)
    };
    assert_eq!(edges("a"), (false, false));
    assert_eq!(edges("a \n"), (false, false));
    assert_eq!(edges(" a\nb"), (false, false));
    assert_eq!(edges("\na"), (false, false));
    assert_eq!(edges(" \t\na"), (true, false));
    assert_eq!(edges("a\n\u{00A0}"), (false, true));
    assert_eq!(edges("  \na\t"), (true, true));
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
    "\u{2003}",
    "\u{1F680}",
];

fn random_text(rng: &mut StdRng, max_chars: usize) -> String {
    let len = rng.gen_range(0..=max_chars);
    (0..len)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())])
        .collect()
}

/// Rewrite `region` the way a model might retype it: swap quotes, dashes and
/// spaces for look-alikes, add or drop trailing spaces before newlines
/// (which also covers a whitespace-only first line), and add or drop
/// whitespace at the end.
fn retype(rng: &mut StdRng, region: &str) -> String {
    let mut out = String::new();
    for c in region.chars() {
        if c == '\n' && rng.gen_bool(0.3) {
            let trimmed = out.trim_end_matches(is_line_space).len();
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
    if rng.gen_bool(0.25) {
        let trimmed = out.trim_end_matches(is_line_space).len();
        out.truncate(trimmed);
    }
    if rng.gen_bool(0.25) {
        out.push_str([" ", "\t", "  ", " \t"][rng.gen_range(0..4)]);
    }
    out
}

fn char_boundaries(text: &str) -> Vec<usize> {
    text.char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(text.len()))
        .collect()
}

fn line_space_len(chars: impl Iterator<Item = char>) -> usize {
    chars
        .take_while(|c| is_line_space(*c))
        .map(char::len_utf8)
        .sum()
}

/// Oracle, independent of `locate`: the range a fuzzy edit of `old`, typed
/// from `content[i..j]`, must replace. The region's own edge whitespace is
/// dropped; where `old` has edge whitespace, the file's whitespace run there
/// is taken instead. `None` when `old` ends in whitespace but the file has
/// neither whitespace nor a line end after the region: that is no match.
fn expected_range(content: &str, i: usize, j: usize, old: &str) -> Option<Range<usize>> {
    let region = &content[i..j];
    let start = match old.split_once('\n') {
        Some((first, _)) if first.chars().all(is_line_space) => {
            let newline = i + region.find('\n').expect("region keeps its newlines");
            let run = line_space_len(content[..newline].chars().rev());
            if first.is_empty() {
                newline
            } else {
                newline - run
            }
        }
        _ => i,
    };
    let core_end = i + region.trim_end_matches(is_line_space).len();
    let end = if old.ends_with(is_line_space) {
        let after = &content[core_end..];
        let run = line_space_len(after.chars());
        let fits = run > 0 || after.is_empty() || after.starts_with('\n');
        fits.then_some(core_end + run)?
    } else {
        core_end
    };
    Some(start..end)
}

#[test]
fn fuzzy_edits_replace_exactly_the_intended_text_and_never_panic() {
    let mut rng = StdRng::seed_from_u64(0x2191);
    let (mut checked, mut widened) = (0usize, 0usize);
    for _ in 0..30_000 {
        let content = random_text(&mut rng, 40);
        let bounds = char_boundaries(&content);
        let i = bounds[rng.gen_range(0..bounds.len())];
        let j = bounds[rng.gen_range(0..bounds.len())];
        let (i, j) = (i.min(j), i.max(j));
        let old = retype(&mut rng, &content[i..j]);
        let needle = fuzzy_normalise(&old).unwrap().text;
        let is_exact = !old.is_empty() && content.contains(old.as_str());
        let expected = expected_range(&content, i, j, &old);
        let case = format!("{content:?} / {old:?} from {i}..{j}");

        let location = locate(&content, &old).unwrap_or_else(|_| panic!("unmappable: {case}"));
        let range = match location {
            Location::Unique(range) => range,
            Location::Ambiguous(_) => continue,
            Location::NotFound => {
                let may_miss = needle.is_empty() || expected.is_none();
                assert!(may_miss && !is_exact, "not found: {case}");
                continue;
            }
        };
        let replaced = content
            .get(range.clone())
            .expect("range on char boundaries");
        let edited = splice_for_test(&content, &range, "<NEW>");
        assert_eq!(edited.get(..range.start), content.get(..range.start));
        assert!(edited.ends_with(&content[range.end..]));
        if is_exact {
            assert_eq!(replaced, old, "{case}");
            continue;
        }
        assert_eq!(fuzzy_normalise(replaced).unwrap().text, needle, "{case}");
        let Some(expected) = expected else {
            continue;
        };
        assert_eq!(range, expected, "{case}");
        checked += 1;
        let region_core = i + content[i..j].trim_end_matches(is_line_space).len();
        widened += usize::from(range.start < i || range.end > region_core);
    }
    assert!(checked > 2_000, "too few fuzzy edits checked: {checked}");
    assert!(
        widened > 200,
        "too few edge-whitespace edits checked: {widened}"
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
