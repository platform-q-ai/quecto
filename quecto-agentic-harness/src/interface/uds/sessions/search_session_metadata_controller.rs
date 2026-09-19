//! Controller of the `search_session_metadata` command (#2010): hands the
//! application's typed request to the single query owner and the typed result
//! to the presenter. No policy: what matches, in which scope, in what order
//! and how fresh are the application's and the adapter's.
use std::sync::Arc;

use crate::application::sessions::dto::{
    SearchSessionMetadataRequest, SearchSessionMetadataResult,
};
use crate::application::sessions::use_cases::SearchSessionMetadata;
use crate::domain::error::DomainError;

pub struct SearchSessionMetadataController {
    search: Arc<SearchSessionMetadata>,
}

impl SearchSessionMetadataController {
    pub fn new(search: Arc<SearchSessionMetadata>) -> Self {
        Self { search }
    }

    pub async fn search(
        &self,
        request: &SearchSessionMetadataRequest,
    ) -> Result<SearchSessionMetadataResult, DomainError> {
        self.search.search(request).await
    }
}

impl std::fmt::Debug for SearchSessionMetadataController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchSessionMetadataController")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "search_session_metadata_controller_tests.rs"]
mod tests;
