//! Scope-aware session listing DTOs (#2001 D3).
//!
//! Metadata rows only: title, opaque key, repository label, path — never
//! transcript body. Local listing is scope-filtered; global search is
//! metadata-only.

use crate::domain::session_home_scope::{
    CanonicalExecutionLocation, RepositoryLabel, SessionHomeScope,
};
use crate::domain::session_identity::SessionIdentity;

/// One catalogue row for picker / search surfaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedSessionRow {
    pub identity: SessionIdentity,
    pub title: String,
    pub message_count: usize,
    pub updated_unix_secs: Option<u64>,
    pub home_scope: SessionHomeScope,
    pub repository_label: Option<RepositoryLabel>,
    pub execution_path: Option<CanonicalExecutionLocation>,
    /// True when the session has never been explicitly associated.
    pub is_legacy_unscoped: bool,
}

impl ScopedSessionRow {
    pub fn opaque_key(&self) -> &str {
        self.identity.runtime_key()
    }
}

/// Local-by-default listing versus global metadata search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeListQuery {
    /// Sessions whose home matches `current` (or grouped worktree members).
    Local { current: SessionHomeScope },
    /// Global metadata search; empty query returns all known rows (including legacy).
    Global { query: String },
}

impl ScopeListQuery {
    pub fn local(current: SessionHomeScope) -> Self {
        Self::Local { current }
    }

    pub fn global(query: impl Into<String>) -> Self {
        Self::Global {
            query: query.into(),
        }
    }

    pub fn is_local(&self) -> bool {
        matches!(self, Self::Local { .. })
    }

    pub fn is_global(&self) -> bool {
        matches!(self, Self::Global { .. })
    }
}

/// Whether a metadata row matches a global search string (title/key/repo/path).
pub fn row_matches_metadata_query(row: &ScopedSessionRow, query: &str) -> bool {
    let q = query.trim();
    if q.is_empty() {
        return true;
    }
    let q_lower = q.to_ascii_lowercase();
    let haystacks = [
        row.opaque_key().to_ascii_lowercase(),
        row.title.to_ascii_lowercase(),
        row.repository_label
            .as_ref()
            .map(|l| l.as_str().to_ascii_lowercase())
            .unwrap_or_default(),
        row.execution_path
            .as_ref()
            .map(|p| p.as_str().to_ascii_lowercase())
            .unwrap_or_default(),
    ];
    haystacks.iter().any(|h| h.contains(&q_lower))
}

/// Whether a row belongs in the local picker for `current` scope.
pub fn row_matches_local_scope(row: &ScopedSessionRow, current: &SessionHomeScope) -> bool {
    if row.is_legacy_unscoped {
        // Legacy unscoped sessions are global-only until explicit association.
        return false;
    }
    match (current, &row.home_scope) {
        (
            SessionHomeScope::Scoped {
                execution_location: cur,
                repository_grouping: cur_g,
            },
            SessionHomeScope::Scoped {
                execution_location: home,
                repository_grouping: home_g,
            },
        ) => {
            if cur == home {
                return true;
            }
            // Related worktrees group together when either side lists the other.
            if let Some(g) = cur_g.as_ref() {
                if g.contains_location(home) {
                    return true;
                }
            }
            if let Some(g) = home_g.as_ref() {
                if g.contains_location(cur) {
                    return true;
                }
            }
            false
        }
        _ => false,
    }
}

#[cfg(test)]
#[path = "scope_listing_tests.rs"]
mod tests;
