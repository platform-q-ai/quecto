//! Pure metadata-search values (#2010): what a query is, what it is matched
//! against and how matches rank. Matching compares *visible text* — what a
//! safe renderer would show — so an invisible or control character can neither
//! hide a record from a query nor smuggle one into a result. Nothing here
//! reads a file, a transcript, Git or a clock; a query is literal text, never
//! a pattern.
use super::session_home::SessionHomeScope;
use super::session_home_text::path_below;
pub use super::session_home_text::{execution_path, group_root, repository_label};
pub use super::session_metadata_text::{display_path, shown_text, visible_text};
pub use super::session_query_refusal::{MAX_QUERY_CHARS, QueryRefusal};
pub use super::session_title_subsequence::rank_of;
use super::session_title_subsequence::{MIN_SUBSEQUENCE_TERM_CHARS, title_holds_subsequence};
use std::path::Path;

/// The metadata a query can match, in rank order: an exact key outranks a
/// title, a title a repository label, a label a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchedField {
    Key,
    Title,
    Repository,
    Path,
    /// A term found nowhere literally, matched as an in-order subsequence of
    /// the title (#2043): the lowest tier.
    TitleFuzzy,
}

impl MatchedField {
    /// The one spelling of the field, shared by every presenter.
    pub fn name(self) -> &'static str {
        match self {
            Self::Key => "key",
            Self::Title => "title",
            Self::Repository => "repository",
            Self::Path => "path",
            Self::TitleFuzzy => "title_fuzzy",
        }
    }
}

/// The searchable metadata of one saved session. No transcript content
/// beyond the listing title exists here, so none can be matched.
#[derive(Debug, Clone, Copy)]
pub struct SessionMetadataFields<'a> {
    pub key: &'a str,
    pub title: &'a str,
    pub home: &'a SessionHomeScope,
    /// Set for a search of the current workspace group only (R1-T12): the
    /// root every row shares, which therefore distinguishes none of them.
    pub local_root: Option<&'a Path>,
}

/// A parsed query: the exact text a key must equal, and the visible terms
/// every one of which must occur in the title, repository label or path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataQuery {
    exact: String,
    terms: Vec<String>,
    /// The terms long enough, as typed, for the title subsequence tier.
    fuzzy: Vec<String>,
}

impl MetadataQuery {
    pub fn parse(raw: &str) -> Result<Self, QueryRefusal> {
        let visible = visible_text(raw);
        let chars = shown_text(raw).chars().count();
        if chars <= MAX_QUERY_CHARS {
            let terms: Vec<String> = visible
                .split(' ')
                .filter(|t| !t.is_empty())
                .map(str::to_string)
                .collect();
            // The fold never adds or removes a space, so the shown terms
            // align with the folded ones; the bound counts shown characters.
            let fuzzy = shown_text(raw)
                .split(' ')
                .filter(|t| !t.is_empty())
                .zip(&terms)
                .filter(|(shown, _)| shown.chars().count() >= MIN_SUBSEQUENCE_TERM_CHARS)
                .map(|(_, term)| term.clone())
                .collect();
            Ok(Self {
                exact: raw.trim().to_string(),
                terms,
                fuzzy,
            })
        } else {
            Err(QueryRefusal::TooLong { chars })
        }
    }

    /// The echo of `raw`: the visible text that is searched, before the
    /// fold — of a refused query, its first [`MAX_QUERY_CHARS`] characters.
    pub fn shown(raw: &str) -> String {
        shown_text(raw).chars().take(MAX_QUERY_CHARS).collect()
    }

    /// A query with nothing visible in it names every session.
    pub fn is_everything(&self) -> bool {
        self.terms.is_empty()
    }

    /// The fields `self` matched, in rank order; `None` when the record is no
    /// match. An everything-query matches with no field.
    pub fn matches(&self, fields: &SessionMetadataFields<'_>) -> Option<Vec<MatchedField>> {
        if self.is_everything() {
            return Some(Vec::new());
        }
        let mut matched = Vec::new();
        // Opaque: the whole key, byte for byte — never a fragment or a fold.
        if fields.key == self.exact {
            matched.push(MatchedField::Key);
        }
        let (label, path) = match fields.local_root {
            None => (repository_label(fields.home), execution_path(fields.home)),
            // Local: the label and the root are every row's; only what lies
            // below the root tells two rows apart (R1-T12).
            Some(root) => (None, path_below(fields.home, root)),
        };
        let texts = [
            (MatchedField::Title, Some(visible_text(fields.title))),
            (MatchedField::Repository, label.map(|l| visible_text(&l))),
            (MatchedField::Path, path.map(|p| visible_text(&p))),
        ];
        let holds =
            |text: &Option<String>, term: &str| text.as_deref().is_some_and(|t| t.contains(term));
        let literal = |term: &str| texts.iter().any(|(_, text)| holds(text, term));
        let fuzzy = |term: &String| {
            self.fuzzy.contains(term)
                && texts[0]
                    .1
                    .as_deref()
                    .is_some_and(|title| title_holds_subsequence(term, title))
        };
        let every_term_found = self.terms.iter().all(|term| literal(term) || fuzzy(term));
        if every_term_found {
            for (field, text) in &texts {
                if self.terms.iter().any(|term| holds(text, term)) && !matched.contains(field) {
                    matched.push(*field);
                }
            }
            if self.terms.iter().any(|term| !literal(term)) {
                matched.push(MatchedField::TitleFuzzy);
            }
        }
        debug_assert!(matched.is_sorted());
        (!matched.is_empty()).then_some(matched)
    }
}

#[cfg(test)]
#[path = "session_metadata_search_tests.rs"]
mod tests;
