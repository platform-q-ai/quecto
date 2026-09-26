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
/// ([`fuzzy_normalise`]) and the match is mapped back to the original bytes.
/// The original range then runs from the first to the last original
/// character behind the matched text: whitespace that normalisation dropped
/// INSIDE that span is replaced with it, whitespace just outside it is kept.
pub(super) fn locate(content: &str, old: &str) -> Result<Location, Unmappable> {
    match count_occurrences_capped(content, old, 2) {
        (1, Some(start)) => return exact_range(content, start, old.len()),
        (count @ 2.., _) => return Ok(Location::Ambiguous(count)),
        (0, None) => {}
        _ => return Err(Unmappable),
    }
    let needle = fuzzy_normalise(old).text;
    let haystack = fuzzy_normalise(content);
    match count_occurrences_capped(&haystack.text, &needle, 2) {
        (0, None) => Ok(Location::NotFound),
        (count @ 2.., _) => Ok(Location::Ambiguous(count)),
        (1, Some(start)) => {
            let original = haystack
                .original_range(content, start..start + needle.len())
                .ok_or(Unmappable)?;
            let proof = prove_fuzzy_range(content, &original, &needle);
            debug_assert_eq!(
                proof,
                Ok(()),
                "{original:?} does not map back to {needle:?}"
            );
            proof?;
            Ok(Location::Unique(original))
        }
        _ => Err(Unmappable),
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
    let proven = fuzzy_normalise(matched).text == needle;
    if proven { Ok(()) } else { Err(Unmappable) }
}

/// Fuzzy-normalised text plus, for each of its bytes, the byte offset in the
/// original text of the character that produced it.
#[derive(Debug)]
pub(super) struct FuzzyText {
    pub(super) text: String,
    source: Vec<usize>,
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
        let start = *self.source.get(range.start)?;
        let last = *self.source.get(range.end.checked_sub(1)?)?;
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
pub(super) fn fuzzy_normalise(s: &str) -> FuzzyText {
    let mut text = String::with_capacity(s.len());
    let mut source = Vec::with_capacity(s.len());
    let mut line_start = 0usize;
    let mut lines = s.split('\n').peekable();
    while let Some(line) = lines.next() {
        for (offset, c) in line.trim_end().char_indices() {
            let mapped = fuzzy_char(c);
            text.push(mapped);
            source.extend(std::iter::repeat_n(line_start + offset, mapped.len_utf8()));
        }
        let newline_at = line_start + line.len();
        if lines.peek().is_some() {
            debug_assert_eq!(s.as_bytes().get(newline_at), Some(&b'\n'));
            text.push('\n');
            source.push(newline_at);
        }
        line_start = newline_at + 1;
    }
    debug_assert_eq!(text.len(), source.len());
    FuzzyText { text, source }
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

/// Count occurrences of `needle` in `haystack`, OVERLAPPING ones included,
/// stopping at `cap`, with the offset of the first one. `aa` is in `aaa`
/// twice: either could be the one meant, so that edit is ambiguous (#2191).
fn count_occurrences_capped(haystack: &str, needle: &str, cap: usize) -> (usize, Option<usize>) {
    if needle.is_empty() {
        return (0, None);
    }
    let mut count = 0usize;
    let mut first_offset: Option<usize> = None;
    let mut start = 0;
    while let Some(pos) = haystack.get(start..).and_then(|rest| rest.find(needle)) {
        let abs_pos = start + pos;
        first_offset.get_or_insert(abs_pos);
        count += 1;
        if count >= cap {
            break;
        }
        let Some(first_char) = needle.chars().next() else {
            break;
        };
        start = abs_pos + first_char.len_utf8();
    }
    (count, first_offset)
}

#[cfg(test)]
#[path = "edit_match_tests.rs"]
mod tests;
