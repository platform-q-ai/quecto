// Locating `oldText` in a file for the `edit` tool: exact match first, then a
// fuzzy fallback that forgives trailing whitespace and typographic characters.
//
// Pure text logic, no I/O. Every range it hands back is a byte range of the
// text it was given, on character boundaries, and has been proven to hold the
// text that matched (#2191).

use std::ops::Range;

/// Where `oldText` is in the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Location {
    /// No match, exact or fuzzy.
    NotFound,
    /// More than one match; the count stops at 2.
    Ambiguous(usize),
    /// Exactly one match: the byte range of the original text to replace.
    Unique(Range<usize>),
}

/// The fuzzy match could not be mapped back to the original text, so the
/// edit is refused rather than spliced in a guessed place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Unmappable;

/// Find `old` in `content`. Both must already be BOM-stripped and in LF form.
///
/// An exact match wins. With no exact match, both sides are fuzzy-normalised
/// ([`fuzzy_normalise`]) and each match is mapped back to the original bytes:
/// from the first to the last original character behind the matched text,
/// so whitespace that normalisation dropped INSIDE that span goes with it.
/// At the edges, whitespace goes with the match only where `old` has
/// whitespace there too ([`Edges`]), so `newText`'s whitespace replaces the
/// file's instead of adding to it. Only matches whose edges fit are counted.
pub(super) fn locate(content: &str, old: &str) -> Result<Location, Unmappable> {
    let exact: Vec<usize> = occurrences(content, old).take(2).collect();
    match exact.as_slice() {
        [start] => return exact_range(content, *start, old.len()),
        [_, _] => return Ok(Location::Ambiguous(2)),
        [] => {}
        _ => return Err(Unmappable),
    }
    let needle = fuzzy_normalise(old)?.text;
    let haystack = fuzzy_normalise(content)?;
    let edges = Edges::of(old);
    let mut found: Vec<Range<usize>> = Vec::with_capacity(2);
    for start in occurrences(&haystack.text, &needle) {
        let core = haystack
            .original_range(content, start..start + needle.len())
            .ok_or(Unmappable)?;
        if let Some(range) = edges.fit(content, core) {
            found.push(range);
        }
        if found.len() == 2 {
            break;
        }
    }
    match found.as_slice() {
        [] => Ok(Location::NotFound),
        [_, _] => Ok(Location::Ambiguous(2)),
        [range] => {
            let proof = prove_fuzzy_range(content, range, &needle);
            debug_assert_eq!(proof, Ok(()), "{range:?} does not map back to {needle:?}");
            proof?;
            Ok(Location::Unique(range.clone()))
        }
        _ => Err(Unmappable),
    }
}

/// Whitespace within a line: everything `trim_end` drops except the newline.
fn is_line_space(c: char) -> bool {
    c.is_whitespace() && c != '\n'
}

/// Byte length of the in-line whitespace run at the front of `chars`.
fn line_space_len(chars: impl Iterator<Item = char>) -> usize {
    chars
        .take_while(|c| is_line_space(*c))
        .map(char::len_utf8)
        .sum()
}

/// The whitespace at the edges of `oldText` that fuzzy normalisation drops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Edges<'a> {
    /// The first line of `oldText` is whitespace only (and a newline follows):
    /// it stands for the file's trailing whitespace before the matched
    /// newline, or for an empty line.
    before: bool,
    /// The whitespace `oldText` ends with (empty when it ends otherwise): it
    /// stands for the same whitespace in the file right after the match, or
    /// for trailing whitespace up to a line end.
    after: &'a str,
}

impl<'a> Edges<'a> {
    fn of(old: &'a str) -> Self {
        let before = old
            .split_once('\n')
            .is_some_and(|(first, _)| !first.is_empty() && first.chars().all(is_line_space));
        let core_len = old.trim_end_matches(is_line_space).len();
        let after = old.get(core_len..).unwrap_or_default();
        Self { before, after }
    }

    /// Widen the core match over the file's whitespace at the edges where
    /// `oldText` has whitespace, or `None` when the edges do not fit.
    fn fit(self, content: &str, core: Range<usize>) -> Option<Range<usize>> {
        let start = if self.before {
            self.fit_start(content, core.start)?
        } else {
            core.start
        };
        let end = if self.after.is_empty() {
            core.end
        } else {
            self.fit_end(content, core.end)?
        };
        content.get(start..end).map(|_| start..end)
    }

    /// The core starts at a newline. Take the file's trailing whitespace
    /// before it; with none there, the newline must end an empty line (or
    /// start the file), else oldText's blank line would join two lines.
    fn fit_start(self, content: &str, core_start: usize) -> Option<usize> {
        let before_core = content.get(..core_start)?;
        let run = line_space_len(before_core.chars().rev());
        let start = core_start.checked_sub(run)?;
        let line_before = content.get(..start)?;
        let is_line_start = line_before.is_empty() || line_before.ends_with('\n');
        (run > 0 || is_line_start).then_some(start)
    }

    /// A run that reaches the line end is trailing whitespace: take it whole.
    /// Otherwise take exactly oldText's trailing whitespace from the front of
    /// the file's run (compared after `fuzzy_char`), or the edge does not fit.
    fn fit_end(self, content: &str, core_end: usize) -> Option<usize> {
        let after_core = content.get(core_end..)?;
        let run = line_space_len(after_core.chars());
        let rest = after_core.get(run..)?;
        let is_trailing = rest.is_empty() || rest.starts_with('\n');
        if is_trailing {
            return core_end.checked_add(run);
        }
        let mut file_run = after_core.chars();
        let mut taken = 0usize;
        for wanted in self.after.chars() {
            let got = file_run
                .next()
                .filter(|c| is_line_space(*c) && fuzzy_char(*c) == fuzzy_char(wanted))?;
            taken += got.len_utf8();
        }
        core_end.checked_add(taken)
    }
}

fn exact_range(content: &str, start: usize, len: usize) -> Result<Location, Unmappable> {
    let range = start..start.checked_add(len).ok_or(Unmappable)?;
    match content.get(range.clone()) {
        Some(_) => Ok(Location::Unique(range)),
        None => Err(Unmappable),
    }
}

/// The original range must sit on character boundaries and normalise to
/// exactly the text that matched; otherwise nothing is written.
fn prove_fuzzy_range(content: &str, range: &Range<usize>, needle: &str) -> Result<(), Unmappable> {
    let matched = content.get(range.clone()).ok_or(Unmappable)?;
    let proven = fuzzy_normalise(matched)?.text == needle;
    if proven { Ok(()) } else { Err(Unmappable) }
}

/// Fuzzy-normalised text plus, for each of its bytes, the byte offset in the
/// original text of the character that produced it. Offsets are `u32` (4
/// bytes per byte of text): the edit tool's 1 MiB file cap keeps them far
/// below `u32::MAX`, and [`fuzzy_normalise`] refuses text that does not fit.
#[derive(Debug)]
pub(super) struct FuzzyText {
    pub(super) text: String,
    source: Vec<u32>,
}

impl FuzzyText {
    /// Map a range of the normalised text back to the original: from the
    /// start of the first source character to the end of the last one.
    /// `None` when the range is empty, out of bounds or off a boundary.
    fn original_range(&self, original: &str, range: Range<usize>) -> Option<Range<usize>> {
        let is_mappable = range.start < range.end
            && self.text.is_char_boundary(range.start)
            && self.text.is_char_boundary(range.end);
        let range = is_mappable.then_some(range)?;
        let start = usize::try_from(*self.source.get(range.start)?).ok()?;
        let last = usize::try_from(*self.source.get(range.end.checked_sub(1)?)?).ok()?;
        let last_char = original.get(last..)?.chars().next()?;
        let end = last.checked_add(last_char.len_utf8())?;
        let is_boundary_range = start <= end && original.get(start..end).is_some();
        is_boundary_range.then_some(start..end)
    }
}

/// Normalise text for fuzzy matching — mirrors Quecto's `normalizeForFuzzyMatch`.
/// The input must already be BOM-stripped and in LF form.
///
/// - Trailing whitespace stripped per line
/// - Smart single quotes (U+2018–U+201B) → `'`
/// - Smart double quotes (U+201C–U+201F) → `"`
/// - Unicode dashes (U+2010–U+2015, U+2212) → `-`
/// - Special/non-breaking spaces → regular ASCII space
pub(super) fn fuzzy_normalise(s: &str) -> Result<FuzzyText, Unmappable> {
    let fits_offset_map = u32::try_from(s.len()).is_ok();
    let s = fits_offset_map.then_some(s).ok_or(Unmappable)?;
    let offset = |at: usize| u32::try_from(at).map_err(|_| Unmappable);
    let mut text = String::with_capacity(s.len());
    let mut source = Vec::with_capacity(s.len());
    let mut line_start = 0usize;
    let mut lines = s.split('\n').peekable();
    while let Some(line) = lines.next() {
        for (at, c) in line.trim_end().char_indices() {
            let mapped = fuzzy_char(c);
            text.push(mapped);
            let from = offset(line_start + at)?;
            source.extend(std::iter::repeat_n(from, mapped.len_utf8()));
        }
        let newline_at = line_start + line.len();
        if lines.peek().is_some() {
            debug_assert_eq!(s.as_bytes().get(newline_at), Some(&b'\n'));
            text.push('\n');
            source.push(offset(newline_at)?);
        }
        line_start = newline_at + 1;
    }
    debug_assert_eq!(text.len(), source.len());
    Ok(FuzzyText { text, source })
}

/// Map a single character to its fuzzy-normalised equivalent.
#[inline]
fn fuzzy_char(c: char) -> char {
    match c {
        // Smart single quotes → straight single quote
        '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' => '\'',
        // Smart double quotes → straight double quote
        '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' => '"',
        // Unicode dashes / minus → ASCII hyphen-minus
        '\u{2010}'..='\u{2015}' | '\u{2212}' => '-',
        // Non-breaking and typographic spaces → regular space
        '\u{00A0}' | '\u{2002}'..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}' => ' ',
        other => other,
    }
}

/// Every start offset of `needle` in `haystack`, OVERLAPPING ones included:
/// `aa` is in `aaa` twice, and either could be the one meant, so that edit
/// is ambiguous (#2191). An empty needle occurs nowhere.
///
/// One linear pass (Knuth-Morris-Pratt over bytes): the fuzzy search reads
/// every candidate and may reject most of them at their edges, so restarting
/// a substring search after each one would be quadratic on periodic text.
/// A UTF-8 needle only matches UTF-8 text on character boundaries.
fn occurrences<'a>(haystack: &'a str, needle: &'a str) -> impl Iterator<Item = usize> + 'a {
    let pattern = needle.as_bytes();
    let fallback = kmp_fallback(pattern);
    let mut text = haystack.as_bytes().iter().enumerate();
    let mut matched = 0usize;
    std::iter::from_fn(move || {
        let last = pattern.len().checked_sub(1)?;
        for (at, byte) in text.by_ref() {
            while matched > 0 && pattern.get(matched) != Some(byte) {
                matched = fallback.get(matched - 1).copied()?;
            }
            if pattern.get(matched) == Some(byte) {
                matched += 1;
            }
            if matched == pattern.len() {
                matched = fallback.get(last).copied()?;
                return Some(at - last);
            }
        }
        None
    })
}

/// For each prefix of `pattern`, the length of its longest proper prefix
/// that is also its suffix.
fn kmp_fallback(pattern: &[u8]) -> Vec<usize> {
    let mut fallback = vec![0usize; pattern.len()];
    let mut len = 0usize;
    for at in 1..pattern.len() {
        while len > 0 && pattern.get(at) != pattern.get(len) {
            len = fallback.get(len - 1).copied().unwrap_or(0);
        }
        if pattern.get(at) == pattern.get(len) {
            len += 1;
        }
        if let Some(slot) = fallback.get_mut(at) {
            *slot = len;
        }
    }
    fallback
}

#[cfg(test)]
#[path = "edit_match_tests.rs"]
mod tests;
