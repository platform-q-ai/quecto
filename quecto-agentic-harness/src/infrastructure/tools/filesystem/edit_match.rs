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

/// The whitespace at the edges of `oldText` that fuzzy normalisation drops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Edges {
    /// The first line of `oldText` is whitespace only (and a newline follows):
    /// it stands for the file's trailing whitespace before the matched newline.
    before: bool,
    /// `oldText` ends in whitespace: it stands for the file's whitespace run
    /// right after the match, or for a line end.
    after: bool,
}

impl Edges {
    fn of(old: &str) -> Self {
        let before = old
            .split_once('\n')
            .is_some_and(|(first, _)| !first.is_empty() && first.chars().all(is_line_space));
        let after = old.chars().next_back().is_some_and(is_line_space);
        Self { before, after }
    }

    /// Widen the core match over the file's whitespace at the edges where
    /// `oldText` has whitespace. `None` when `oldText` ends in whitespace but
    /// the file has neither whitespace nor a line end there: not a match.
    fn fit(self, content: &str, core: Range<usize>) -> Option<Range<usize>> {
        let before_core = content.get(..core.start)?;
        let after_core = content.get(core.end..)?;
        let start = if self.before {
            let run: usize = before_core
                .chars()
                .rev()
                .take_while(|c| is_line_space(*c))
                .map(char::len_utf8)
                .sum();
            core.start - run
        } else {
            core.start
        };
        let end = if self.after {
            let run: usize = after_core
                .chars()
                .take_while(|c| is_line_space(*c))
                .map(char::len_utf8)
                .sum();
            let is_line_end = after_core.is_empty() || after_core.starts_with('\n');
            let fits = run > 0 || is_line_end;
            fits.then_some(core.end + run)?
        } else {
            core.end
        };
        content.get(start..end).map(|_| start..end)
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
fn occurrences<'a>(haystack: &'a str, needle: &'a str) -> impl Iterator<Item = usize> + 'a {
    let step = needle.chars().next().map_or(0, char::len_utf8);
    let mut from = (step > 0).then_some(0usize);
    std::iter::from_fn(move || {
        let start = from?;
        let found = haystack
            .get(start..)
            .and_then(|rest| rest.find(needle))
            .map(|pos| start + pos);
        from = found.map(|at| at + step);
        found
    })
}

#[cfg(test)]
#[path = "edit_match_tests.rs"]
mod tests;
