//! The summary walk behind `SessionStore::list` (#1861): one pass over the
//! layout's flat directory, the `.json` allowlist and the identity-prefix
//! skip applied before any read, one lightweight header parse per admitted
//! record, tolerant skipping of what cannot be read or summarised, newest
//! first. Summary-only: nothing here loads a session in full.
use crate::application::sessions::dto::SessionListQuery;
use crate::domain::error::DomainError;
use crate::domain::session::SessionSummary;

use super::super::session_layout::FlatSessionLayout;

#[path = "session_store_list_scan.rs"]
mod session_store_list_scan;

#[derive(Debug, Default)]
pub(super) struct SummaryCache {
    pub(super) reads: usize,
    pub(super) entries: std::collections::BTreeMap<std::path::PathBuf, (Vec<u64>, SessionSummary)>,
}

pub(super) async fn list_summaries(
    layout: &FlatSessionLayout,
    query: &SessionListQuery,
    cache: std::sync::Arc<std::sync::Mutex<SummaryCache>>,
) -> Result<Vec<SessionSummary>, DomainError> {
    let layout = layout.clone();
    let query = query.clone();
    tokio::task::spawn_blocking(move || session_store_list_scan::scan(&layout, &query, &cache))
        .await
        .map_err(|e| DomainError::Session(e.to_string()))?
}
