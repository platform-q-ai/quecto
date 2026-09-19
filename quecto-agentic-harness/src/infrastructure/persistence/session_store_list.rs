//! The summary walk behind `SessionStore::list` (#1861): one pass over the
//! layout's flat directory, the `.json` allowlist and the identity-prefix
//! skip applied before any read, one lightweight header parse per admitted
//! record, tolerant skipping of what cannot be read or summarised (each skip
//! named in the walk's answer), newest first. Summary-only: nothing here loads a session in full.
use crate::application::sessions::dto::SessionListQuery;
use crate::domain::error::DomainError;

use super::super::session_layout::FlatSessionLayout;

#[path = "session_store_list_scan.rs"]
mod session_store_list_scan;

#[path = "session_store_list_index.rs"]
mod session_store_list_index;
pub(super) use session_store_list_index::SummaryCache;
pub(super) use session_store_list_scan::Walk;

pub(super) async fn walk(
    layout: &FlatSessionLayout,
    query: &SessionListQuery,
    cache: std::sync::Arc<std::sync::Mutex<SummaryCache>>,
) -> Result<Walk, DomainError> {
    let (layout, query) = (layout.clone(), query.clone());
    tokio::task::spawn_blocking(move || session_store_list_scan::scan(&layout, &query, &cache))
        .await
        .map_err(|e| DomainError::Session(e.to_string()))?
}
