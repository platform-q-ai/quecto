//! Pure metadata-search values (#2010): what a query is, what it is matched
//! against and how matches rank. Matching compares *visible text* — what a
//! safe renderer would show — so an invisible or control character can neither
//! hide a record from a query nor smuggle one into a result. Nothing here
//! reads a file, a transcript, Git or a clock; a query is literal text, never
//! a pattern.
use super::session_home::{SessionHomeScope, WorkspaceGroup};

/// The longest query, in visible characters, that is searched at all.
pub const MAX_QUERY_CHARS: usize = 256;

/// The metadata a query can match, in rank order: an exact key outranks a
/// title, a title a repository label, a label a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchedField {
    Key,
    Title,
    Repository,
    Path,
}

impl MatchedField {
    /// The one spelling of the field, shared by every presenter.
    pub fn name(self) -> &'static str {
        match self {
            Self::Key => "key",
            Self::Title => "title",
            Self::Repository => "repository",
            Self::Path => "path",
        }
    }
}

/// Why a query is answered with no rows instead of being searched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryRefusal {
    /// More visible characters than [`MAX_QUERY_CHARS`]: a prefix of it would
    /// match records the whole query does not name.
    TooLong { chars: usize },
}

impl std::fmt::Display for QueryRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLong { chars } => write!(
                f,
                "query too long: {chars} characters (at most {MAX_QUERY_CHARS} are searched)"
            ),
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
}

/// A parsed query: the exact text a key must equal, and the visible terms
/// every one of which must occur in the title, repository label or path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataQuery {
    exact: String,
    terms: Vec<String>,
}

impl MetadataQuery {
    pub fn parse(raw: &str) -> Result<Self, QueryRefusal> {
        let visible = visible_text(raw);
        let chars = visible.chars().count();
        if chars <= MAX_QUERY_CHARS {
            Ok(Self {
                exact: raw.trim().to_string(),
                terms: visible
                    .split(' ')
                    .filter(|t| !t.is_empty())
                    .map(str::to_string)
                    .collect(),
            })
        } else {
            Err(QueryRefusal::TooLong { chars })
        }
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
        let texts = [
            (MatchedField::Title, Some(visible_text(fields.title))),
            (
                MatchedField::Repository,
                repository_label(fields.home).map(|l| visible_text(&l)),
            ),
            (
                MatchedField::Path,
                execution_path(fields.home).map(|p| visible_text(&p)),
            ),
        ];
        let holds =
            |text: &Option<String>, term: &str| text.as_deref().is_some_and(|t| t.contains(term));
        let every_term_found = self
            .terms
            .iter()
            .all(|term| texts.iter().any(|(_, text)| holds(text, term)));
        if every_term_found {
            for (field, text) in &texts {
                if self.terms.iter().any(|term| holds(text, term)) && !matched.contains(field) {
                    matched.push(*field);
                }
            }
        }
        matched.sort();
        (!matched.is_empty()).then_some(matched)
    }
}

/// The text a safe renderer would show, folded for comparison: every control
/// and invisible format character dropped, Unicode lower-cased, whitespace
/// runs collapsed to one space and trimmed. Code points are compared as they
/// are stored: a composed and a decomposed spelling of one glyph are two
/// texts (the harness carries no normalization tables off macOS).
pub fn visible_text(raw: &str) -> String {
    let folded: String = raw
        .chars()
        .filter(|ch| !invisible(*ch))
        .flat_map(char::to_lowercase)
        .collect();
    folded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Controls that are not whitespace, and the format characters that reorder,
/// hide or split text (bidi controls, zero-width characters, the soft hyphen,
/// invisible operators, tags, the byte-order mark).
fn invisible(ch: char) -> bool {
    (ch.is_control() && !ch.is_whitespace())
        || matches!(ch,
            '\u{ad}' | '\u{34f}' | '\u{61c}' | '\u{180b}'..='\u{180f}'
            | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}'
            | '\u{feff}' | '\u{fff9}'..='\u{fffb}' | '\u{e0000}'..='\u{e007f}')
}

/// The label of the repository or folder a home belongs to: the directory
/// holding a `.git` common dir, a bare repository's name without `.git`, or
/// the folder's own name. Related worktrees share one label. Lossy for a
/// non-UTF-8 name; `None` for a home without a directory.
pub fn repository_label(home: &SessionHomeScope) -> Option<String> {
    let SessionHomeScope::Scoped(home) = home else {
        return None;
    };
    let named = match &home.group {
        WorkspaceGroup::Git { common_dir } if common_dir.ends_with(".git") => {
            common_dir.parent()?
        }
        WorkspaceGroup::Git { common_dir } => common_dir.as_path(),
        WorkspaceGroup::Folder { directory } => directory.as_path(),
    };
    let name = named.file_name()?.to_string_lossy();
    Some(name.strip_suffix(".git").unwrap_or(&name).to_string())
}

/// The execution directory a home records, lossy for a non-UTF-8 path.
pub fn execution_path(home: &SessionHomeScope) -> Option<String> {
    match home {
        SessionHomeScope::Scoped(home) => Some(home.execution_dir.to_string_lossy().into_owned()),
        SessionHomeScope::LegacyUnscoped | SessionHomeScope::Unavailable(_) => None,
    }
}

#[cfg(test)]
#[path = "session_metadata_search_tests.rs"]
mod tests;
