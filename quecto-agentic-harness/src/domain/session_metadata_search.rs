//! Pure metadata-search values (#2010): what a query is, what it is matched
//! against and how matches rank. Matching compares *visible text* — what a
//! safe renderer would show — so an invisible or control character can neither
//! hide a record from a query nor smuggle one into a result. Nothing here
//! reads a file, a transcript, Git or a clock; a query is literal text, never
//! a pattern.
use super::session_home::{SessionHome, SessionHomeScope, WorkspaceGroup};
pub use super::session_metadata_text::{display_path, visible_text};
use std::path::{Path, PathBuf};

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
    /// A request field that must be a number was something else (R1-H5):
    /// answered — correlated — rather than dropped as undecodable.
    NotANumber { field: &'static str },
}

impl std::fmt::Display for QueryRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLong { chars } => write!(
                f,
                "query too long: {chars} characters (at most {MAX_QUERY_CHARS} are searched)"
            ),
            Self::NotANumber { field } => write!(f, "{field} must be a number"),
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
        // Pushed in rank order already: the key, then `texts` in field order.
        debug_assert!(matched.is_sorted());
        (!matched.is_empty()).then_some(matched)
    }
}

/// The label of the repository or folder a home belongs to: the name of its
/// group's root ([`group_root`]), a bare repository's without `.git`. Related
/// worktrees share one label. `None` for a home without a directory.
pub fn repository_label(home: &SessionHomeScope) -> Option<String> {
    let SessionHomeScope::Scoped(home) = home else {
        return None;
    };
    let name = display_path(Path::new(group_root(home)?.file_name()?));
    Some(name.strip_suffix(".git").unwrap_or(&name).to_string())
}

/// The directory a workspace group is rooted at: the directory holding a
/// `.git` common dir, a bare repository itself, or the folder.
pub fn group_root(home: &SessionHome) -> Option<&Path> {
    match &home.group {
        WorkspaceGroup::Git { common_dir } if common_dir.ends_with(".git") => common_dir.parent(),
        WorkspaceGroup::Git { common_dir } => Some(common_dir.as_path()),
        WorkspaceGroup::Folder { directory } => Some(directory.as_path()),
    }
}

/// The execution directory a home records, spelled by [`display_path`].
pub fn execution_path(home: &SessionHomeScope) -> Option<String> {
    match home {
        SessionHomeScope::Scoped(home) => Some(display_path(&home.execution_dir)),
        SessionHomeScope::LegacyUnscoped | SessionHomeScope::Unavailable(_) => None,
    }
}

/// What `home`'s execution directory does not share with `root`: the path
/// below it, or — for a linked worktree outside it — below their common
/// ancestor. `None` when nothing is left (the root itself) or no directory.
fn path_below(home: &SessionHomeScope, root: &Path) -> Option<String> {
    let SessionHomeScope::Scoped(home) = home else {
        return None;
    };
    let shared = home.execution_dir.components().zip(root.components());
    let shared = shared.take_while(|(a, b)| a == b).count();
    let below: PathBuf = home.execution_dir.components().skip(shared).collect();
    (!below.as_os_str().is_empty()).then(|| display_path(&below))
}

#[cfg(test)]
#[path = "session_metadata_search_tests.rs"]
mod tests;
