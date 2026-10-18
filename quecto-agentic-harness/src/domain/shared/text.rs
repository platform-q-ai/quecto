// Shared UTF-8-safe text truncation core (#996, PR #999 review).
//
// Three previews (grep line truncation, audit content previews, context-pruning
// stubs) share the same bounded-scan idiom: walk at most `max_chars` characters
// with `char_indices().nth()` so a short preview never scans a huge string, and
// cut only on a character boundary. The boundary math lives here once so a fix
// cannot drift between copies.

use std::borrow::Cow;

/// Truncate `s` to at most `max_chars` characters, appending `suffix` when
/// truncated. On truncation, the first `keep` characters are retained (callers
/// choose whether the suffix counts toward the budget). Safe for multi-byte
/// UTF-8 — never splits a character. Returns `Cow::Borrowed` when `s` fits.
pub fn truncate_chars<'a>(s: &'a str, max_chars: usize, keep: usize, suffix: &str) -> Cow<'a, str> {
    // Fast path: byte length ≤ max_chars guarantees ≤ max_chars characters
    // (every char is ≥ 1 byte) — no scan at all for short strings.
    if s.len() <= max_chars {
        return Cow::Borrowed(s);
    }
    // Bounded scan: `nth(max_chars)` yields the byte offset of the first
    // character past the limit (or None if the string fits), so we never
    // count the whole (possibly huge) string just to build a short preview.
    match s.char_indices().nth(max_chars) {
        None => Cow::Borrowed(s),
        Some(_) => {
            let end = s.char_indices().nth(keep).map_or(s.len(), |(idx, _)| idx);
            Cow::Owned(format!("{}{}", &s[..end], suffix))
        }
    }
}
