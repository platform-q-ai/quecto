//! Typed global metadata search (#2010): the query as the client typed it,
//! stable-key rows, the freshness of the index they came from, and the query
//! generation a client uses to discard an answer it no longer wants. Domain
//! values only — no wire field, no adapter record.
use super::{ListedSession, SessionListScope};
use crate::domain::session_metadata_search::{MatchedField, QueryRefusal};

/// The client's own counter of the query an answer belongs to, echoed
/// unchanged: the application attaches no meaning to it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct QueryGeneration(pub u64);

/// How many rows an answer may carry: 1 to [`SearchLimit::MAX`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchLimit(usize);

impl SearchLimit {
    pub const DEFAULT: usize = 200;
    pub const MAX: usize = 500;

    /// The requested limit brought into range; absent is the default.
    pub fn clamped(requested: Option<u64>) -> Self {
        let requested =
            requested.map_or(Self::DEFAULT, |n| usize::try_from(n).unwrap_or(Self::MAX));
        Self(requested.clamp(1, Self::MAX))
    }

    pub fn get(self) -> usize {
        self.0
    }
}

impl Default for SearchLimit {
    fn default() -> Self {
        Self::clamped(None)
    }
}

#[derive(Debug, Clone, Default)]
pub struct SearchSessionMetadataRequest {
    /// Literal text, exactly as typed; the domain decides what is visible.
    pub query: String,
    pub scope: SessionListScope,
    pub generation: QueryGeneration,
    pub limit: SearchLimit,
}

/// One match: the same row a listing shows (so its `home_version()` is the
/// listing's token), what matched, and the label of its repository or folder.
#[derive(Debug, Clone)]
pub struct SessionMetadataRow {
    pub session: ListedSession,
    pub repository_label: Option<String>,
    pub matched: Vec<MatchedField>,
}

/// What the answer is worth: diagnostics of the authority walk and whether
/// the derived index had to be recovered for it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchFreshness {
    pub diagnostics: Vec<String>,
    pub rebuilt: bool,
}

#[derive(Debug, Clone)]
pub struct SearchSessionMetadataResult {
    pub generation: QueryGeneration,
    pub scope: SessionListScope,
    /// Best first: rank of the best matched field, then newest, then key.
    pub rows: Vec<SessionMetadataRow>,
    /// Matches in scope before the limit was applied.
    pub total_matches: usize,
    /// Sessions in scope the query was matched against.
    pub searched: usize,
    /// Why nothing was searched; the rows are then empty.
    pub refused: Option<QueryRefusal>,
    pub freshness: SearchFreshness,
}

impl SearchSessionMetadataResult {
    /// The answer to a request that is not searched: no rows, and why.
    pub fn refused(request: &SearchSessionMetadataRequest, refusal: QueryRefusal) -> Self {
        Self {
            generation: request.generation,
            scope: request.scope,
            rows: Vec::new(),
            total_matches: 0,
            searched: 0,
            refused: Some(refusal),
            freshness: SearchFreshness::default(),
        }
    }

    pub fn truncated(&self) -> bool {
        self.total_matches > self.rows.len()
    }
}

#[cfg(test)]
#[path = "search_session_metadata_tests.rs"]
mod tests;
