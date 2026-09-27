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
    assert_eq!(unique("a\u{2019}\u{00A0}\tb", "a' "), 0..6);
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
    fn edges(old: &str) -> (bool, &str) {
        let e = Edges::of(old);
        (e.before, e.after)
    }
    assert_eq!(edges("a"), (false, ""));
    assert_eq!(edges("a \n"), (false, ""));
    assert_eq!(edges(" a\nb"), (false, ""));
    assert_eq!(edges("\na"), (false, ""));
    assert_eq!(edges(" \t\na"), (true, ""));
    assert_eq!(edges("a\n\u{00A0}"), (false, "\u{00A0}"));
    assert_eq!(edges("  \na \t"), (true, " \t"));
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

// --- cost ---

/// #2191 review: candidates that do not fit must not make the search
/// quadratic. A periodic 1 MiB file with a long needle that matches
/// everywhere except at its whitespace edge.
#[test]
fn a_periodic_file_is_searched_in_linear_time() {
    let content = "ab".repeat(512 * 1024);
    let old = format!("{}a ", "ab".repeat(5_000));
    let started = std::time::Instant::now();
    let location = locate(&content, &old);
    let took = started.elapsed();
    assert_eq!(location, Ok(Location::NotFound));
    assert!(took < std::time::Duration::from_secs(1), "took {took:?}");
}

// --- property: every fuzzy edit replaces exactly what the spec says ---
//
// The oracle below is written from the spec, independently of the code
// under test: its own whitespace set, its own normaliser (a plain char loop)
// and a brute-force search over every range of the file.

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

/// The alphabet's in-line whitespace, listed by hand.
fn spec_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\u{00A0}' | '\u{2003}' | '\u{3000}')
}

/// The alphabet's look-alikes, listed by hand.
fn spec_char(c: char) -> char {
    match c {
        '\u{2018}' | '\u{2019}' => '\'',
        '\u{201C}' | '\u{201D}' => '"',
        '\u{2013}' | '\u{2014}' => '-',
        '\u{00A0}' | '\u{2003}' | '\u{3000}' => ' ',
        other => other,
    }
}

/// Spec normaliser: per line, drop trailing in-line whitespace, then map
/// look-alikes. A plain loop over chars.
fn spec_normalise(text: &str) -> String {
    let mut out = String::new();
    let mut pending = String::new();
    for c in text.chars() {
        if c == '\n' {
            pending.clear();
            out.push('\n');
        } else if spec_space(c) {
            pending.push(spec_char(c));
        } else {
            out.push_str(&pending);
            pending.clear();
            out.push(spec_char(c));
        }
    }
    out
}

fn spec_boundaries(text: &str) -> Vec<usize> {
    text.char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(text.len()))
        .collect()
}

/// Byte length of the leading run of spec whitespace.
fn spec_run(chars: impl Iterator<Item = char>) -> usize {
    let mut len = 0;
    for c in chars {
        if !spec_space(c) {
            break;
        }
        len += c.len_utf8();
    }
    len
}

/// Is `slice` a "core" match: it normalises to the needle and does not
/// carry whitespace that normalisation drops at either edge?
fn spec_is_core(slice: &str, needle: &str) -> bool {
    let first_line_blank = match slice.find('\n') {
        Some(nl) => nl > 0 && slice[..nl].chars().all(spec_space),
        None => false,
    };
    let ends_in_space = slice.chars().last().is_some_and(spec_space);
    !slice.is_empty() && !first_line_blank && !ends_in_space && spec_normalise(slice) == needle
}

/// Widen a core range at the edges where `old` has whitespace, per the spec.
fn spec_fit(content: &str, core: Range<usize>, old: &str) -> Option<Range<usize>> {
    let mut start = core.start;
    let old_first_line_blank = match old.find('\n') {
        Some(nl) => nl > 0 && old[..nl].chars().all(spec_space),
        None => false,
    };
    if old_first_line_blank {
        let run = spec_run(content[..core.start].chars().rev());
        start = core.start - run;
        let line_start = start == 0 || content[..start].ends_with('\n');
        if run == 0 && !line_start {
            return None;
        }
    }
    let old_tail: Vec<char> = old.chars().rev().take_while(|c| spec_space(*c)).collect();
    let mut end = core.end;
    if !old_tail.is_empty() {
        let after = &content[core.end..];
        let run = spec_run(after.chars());
        let reaches_line_end = after[run..].is_empty() || after[run..].starts_with('\n');
        if reaches_line_end {
            end += run;
        } else {
            let file: Vec<char> = after.chars().take(old_tail.len()).collect();
            let wanted = old_tail.iter().rev();
            let same = file.len() == old_tail.len()
                && file
                    .iter()
                    .zip(wanted)
                    .all(|(f, w)| spec_space(*f) && spec_char(*f) == spec_char(*w));
            if !same {
                return None;
            }
            end += file.iter().map(|c| c.len_utf8()).sum::<usize>();
        }
    }
    Some(start..end)
}

/// Brute force: what `locate` must return, from the spec.
fn spec_locate(content: &str, old: &str) -> Location {
    let bounds = spec_boundaries(content);
    let exact: Vec<Range<usize>> = bounds
        .iter()
        .filter(|s| !old.is_empty() && content[**s..].starts_with(old))
        .map(|s| *s..*s + old.len())
        .collect();
    let needle = spec_normalise(old);
    let fitting: Vec<Range<usize>> = match exact.is_empty() && !needle.is_empty() {
        true => bounds
            .iter()
            .flat_map(|s| bounds.iter().map(move |e| *s..*e))
            .filter(|r| r.start < r.end && spec_is_core(&content[r.clone()], &needle))
            .filter_map(|core| spec_fit(content, core, old))
            .collect(),
        false => exact,
    };
    match fitting.as_slice() {
        [] => Location::NotFound,
        [one] => Location::Unique(one.clone()),
        many => Location::Ambiguous(many.len().min(2)),
    }
}

fn random_text(rng: &mut StdRng, max_chars: usize) -> String {
    let len = rng.gen_range(0..=max_chars);
    (0..len)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())])
        .collect()
}

/// Rewrite `region` the way a model might retype it: swap look-alikes, add
/// or drop trailing spaces before newlines (which also covers a blank first
/// line), and cut, drop or add whitespace at the end.
fn retype(rng: &mut StdRng, region: &str) -> String {
    let mut out = String::new();
    for c in region.chars() {
        if c == '\n' && rng.gen_bool(0.3) {
            while out.ends_with(spec_space) {
                out.pop();
            }
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
    match rng.gen_range(0..4) {
        0 => {
            while out.ends_with(spec_space) {
                out.pop();
            }
        }
        1 if out.ends_with(spec_space) => {
            out.pop();
        }
        2 => out.push_str([" ", "\t", "  ", " \t"][rng.gen_range(0..4)]),
        _ => {}
    }
    out
}

#[test]
fn every_edit_location_matches_the_brute_force_spec() {
    let mut rng = StdRng::seed_from_u64(0x2191);
    let (mut fuzzy_unique, mut widened, mut ambiguous) = (0usize, 0usize, 0usize);
    for _ in 0..6_000 {
        let content = random_text(&mut rng, 24);
        let bounds = spec_boundaries(&content);
        let i = bounds[rng.gen_range(0..bounds.len())];
        let j = bounds[rng.gen_range(0..bounds.len())];
        let (i, j) = (i.min(j), i.max(j));
        let old = retype(&mut rng, &content[i..j]);
        let case = format!("{content:?} / {old:?}");

        let got = locate(&content, &old).unwrap_or_else(|_| panic!("unmappable: {case}"));
        assert_eq!(got, spec_locate(&content, &old), "{case}");

        if let Location::Unique(range) = &got {
            let edited = format!("{}<NEW>{}", &content[..range.start], &content[range.end..]);
            assert!(edited.starts_with(&content[..range.start]), "{case}");
            assert!(edited.ends_with(&content[range.end..]), "{case}");
            let is_fuzzy = content[range.clone()] != old;
            fuzzy_unique += usize::from(is_fuzzy);
            let trimmed_end = range.start + content[range.clone()].trim_end().len();
            widened += usize::from(is_fuzzy && (range.end > trimmed_end));
        }
        ambiguous += usize::from(matches!(got, Location::Ambiguous(_)));
    }
    assert!(fuzzy_unique > 500, "too few fuzzy edits: {fuzzy_unique}");
    assert!(widened > 50, "too few edge-widened edits: {widened}");
    assert!(ambiguous > 200, "too few ambiguous cases: {ambiguous}");
}
