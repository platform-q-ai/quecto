//! The discovery handles of one loop (#2009, #2010): the composed list and
//! metadata-search controllers, held together because both answer the resume
//! picker from the same home context. Plain handles: composition builds the
//! controllers, dispatch calls them, and nothing here decides anything.
use std::sync::Arc;

use crate::application::sessions::dto::{
    ListSessionsResult, SearchSessionMetadataRequest, SearchSessionMetadataResult, SessionListScope,
};
use crate::domain::error::DomainError;
use crate::interface::uds::sessions::controller::ListSessionsController;
use crate::interface::uds::sessions::search_session_metadata_controller::SearchSessionMetadataController;

#[derive(Clone, Debug)]
pub struct SessionDiscoveryHandles {
    list: Arc<ListSessionsController>,
    search: Arc<SearchSessionMetadataController>,
}

impl SessionDiscoveryHandles {
    pub fn new(
        list: Arc<ListSessionsController>,
        search: Arc<SearchSessionMetadataController>,
    ) -> Self {
        Self { list, search }
    }

    /// `list_sessions`: every saved session in `scope`.
    pub async fn list(&self, scope: SessionListScope) -> Result<ListSessionsResult, DomainError> {
        self.list.list(scope).await
    }

    /// `search_session_metadata`: the sessions whose metadata matches.
    pub async fn search(
        &self,
        request: &SearchSessionMetadataRequest,
    ) -> Result<SearchSessionMetadataResult, DomainError> {
        self.search.search(request).await
    }
}

#[cfg(test)]
#[path = "uds_discovery_handles_tests.rs"]
mod tests;
