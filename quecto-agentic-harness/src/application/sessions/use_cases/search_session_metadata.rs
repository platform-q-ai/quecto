//! Search saved-session metadata (#2010): the one query behind the UDS
//! `search_session_metadata` command and the TUI picker's search box.
//!
//! The application owns the scope, the match, the order, the limit and the
//! advisory eligibility; the catalogue adapter owns freshness (every answer is
//! validated against authority, as a listing is) and the promise that no
//! transcript is read to match; the domain owns what a query means. A row is a
//! listing row: selecting one still goes through `ResumeSavedSession`, which
//! re-checks the home version under its claim — nothing here restores.
use crate::application::sessions::dto::{
    ListedSession, SearchFreshness, SearchSessionMetadataRequest, SearchSessionMetadataResult,
    SessionListScope, SessionMetadataRow,
};
use crate::application::sessions::ports::session_home::SessionMetadataRecord;
use crate::application::sessions::session_home::SessionHomeContext;
use crate::domain::error::DomainError;
use crate::domain::session_home::{HomeAdmission, SessionHome, SessionHomeScope};
use crate::domain::session_metadata_search::{
    MatchedField, MetadataQuery, SessionMetadataFields, group_root, repository_label,
};

/// The metadata search over the home catalogue and workspace discovery.
pub struct SearchSessionMetadata {
    home: SessionHomeContext,
}

impl SearchSessionMetadata {
    pub fn new(home: SessionHomeContext) -> Self {
        Self { home }
    }

    /// The sessions in `request.scope` whose metadata matches, best first.
    pub async fn search(
        &self,
        request: &SearchSessionMetadataRequest,
    ) -> Result<SearchSessionMetadataResult, DomainError> {
        let query = match MetadataQuery::parse(&request.query) {
            Ok(query) => query,
            Err(refusal) => return Ok(SearchSessionMetadataResult::refused(request, refusal)),
        };
        let mut result = SearchSessionMetadataResult {
            generation: request.generation,
            scope: request.scope,
            rows: Vec::new(),
            total_matches: 0,
            searched: 0,
            refused: None,
            freshness: SearchFreshness::default(),
        };
        let snapshot = self.home.catalogue.metadata().await?;
        result.freshness = SearchFreshness {
            diagnostics: snapshot.diagnostics,
            rebuilt: snapshot.rebuilt,
        };
        let current = match self.home.current().await {
            Ok(current) => Some(current),
            Err(error) => {
                result.freshness.diagnostics.push(error.to_string());
                None
            }
        };
        // Local rows all share the current group's root (R1-T12).
        let local_root = match (request.scope, current.as_ref()) {
            (SessionListScope::Local, Some(current)) => group_root(current),
            _ => None,
        };
        let mut matches: Vec<Match> = Vec::new();
        for record in snapshot.records {
            if in_scope(request.scope, &record.home, current.as_ref()) {
                result.searched += 1;
                let fields = SessionMetadataFields {
                    key: &record.summary.key,
                    title: &record.summary.title,
                    home: &record.home,
                    local_root,
                };
                if let Some(matched) = query.matches(&fields) {
                    matches.push((matched, record));
                }
            }
        }
        result.total_matches = matches.len();
        result.rows = matched_rows(matches, current.as_ref(), request.limit.get());
        debug_assert!(result.rows.len() <= request.limit.get());
        Ok(result)
    }
}

type Match = (Vec<MatchedField>, SessionMetadataRecord);

/// Best first — the best matched field, then newest (undated last), then
/// key: a total order, so equal inputs give one answer — cut to `limit`.
/// Eligibility is decided for the rows that are shown only.
fn matched_rows(
    mut matches: Vec<Match>,
    current: Option<&SessionHome>,
    limit: usize,
) -> Vec<SessionMetadataRow> {
    matches.sort_by(|(a_fields, a), (b_fields, b)| {
        (
            a_fields.first(),
            std::cmp::Reverse(a.summary.updated_unix_secs),
            &a.summary.key,
        )
            .cmp(&(
                b_fields.first(),
                std::cmp::Reverse(b.summary.updated_unix_secs),
                &b.summary.key,
            ))
    });
    matches.truncate(limit);
    matches
        .into_iter()
        .map(|(matched, record)| SessionMetadataRow {
            repository_label: repository_label(&record.home),
            matched,
            session: ListedSession {
                resume_eligible: eligible(&record.home, current),
                summary: record.summary,
                home: record.home,
            },
        })
        .collect()
}

/// Global is every session; Local is the current workspace group, and
/// without current workspace facts it is nothing — never everything.
fn in_scope(
    scope: SessionListScope,
    home: &SessionHomeScope,
    current: Option<&SessionHome>,
) -> bool {
    match (scope, home, current) {
        (SessionListScope::Global, _, _) => true,
        (SessionListScope::Local, SessionHomeScope::Scoped(saved), Some(current)) => {
            saved.group == current.group
        }
        (SessionListScope::Local, _, _) => false,
    }
}

/// Advisory eligibility by the one domain rule, with the discovery of this
/// directory the query already made as the observation: it admits only a
/// saved home that IS the current one (execution directory included), so a
/// home saved anywhere else never admits and nothing is discovered for it.
/// Resume still admits under its claim.
fn eligible(home: &SessionHomeScope, current: Option<&SessionHome>) -> bool {
    match (home, current) {
        (SessionHomeScope::Scoped(saved), Some(current)) => {
            SessionHome::admission(saved, current, current) == HomeAdmission::Eligible
        }
        _ => false,
    }
}

impl std::fmt::Debug for SearchSessionMetadata {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchSessionMetadata")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "search_session_metadata_tests.rs"]
mod tests;
