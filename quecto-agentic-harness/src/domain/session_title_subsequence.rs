//! The lowest tier of the metadata query (#2043): a term found nowhere
//! literally may match a session's TITLE as an in-order subsequence of its
//! visible, case-folded text (`fxbg` ⊂ "fix bug") — never the key, the
//! repository label or the path (paths are long: almost any short
//! subsequence matches; keys are opaque and exact by contract). Only for
//! terms of at least three characters as typed, like the length bound; one
//! pass over the folded title, no transcript content, no new index. A row
//! that needed the tier for any term ranks by it, below every literal match.
use super::session_metadata_search::MatchedField;

/// Shown characters a term needs before the tier is tried on it.
pub const MIN_SUBSEQUENCE_TERM_CHARS: usize = 3;

/// Whether every character of `term` occurs in `title`, in order — both
/// already visible and case-folded, compared code point by code point.
pub fn title_holds_subsequence(term: &str, title: &str) -> bool {
    let mut title = title.chars();
    !term.is_empty() && term.chars().all(|wanted| title.any(|have| have == wanted))
}

/// The tier a row ranks by: the fuzzy one if any term needed it, else its
/// best literal field (the fields arrive in rank order).
pub fn rank_of(matched: &[MatchedField]) -> Option<MatchedField> {
    if matched.contains(&MatchedField::TitleFuzzy) {
        Some(MatchedField::TitleFuzzy)
    } else {
        matched.first().copied()
    }
}

#[cfg(test)]
#[path = "session_title_subsequence_tests.rs"]
mod tests;
