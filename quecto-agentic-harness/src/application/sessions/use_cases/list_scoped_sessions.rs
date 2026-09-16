//! List sessions for local scope or global metadata search (#2001 D3).

use std::sync::Arc;

use crate::application::sessions::dto::scope_listing::{ScopeListQuery, ScopedSessionRow};
use crate::application::sessions::ports::SessionScopeCatalogue;
use crate::domain::error::DomainError;

/// Application owner of scope-aware session listing.
pub struct ListScopedSessions {
    catalogue: Arc<dyn SessionScopeCatalogue>,
}

impl ListScopedSessions {
    pub fn new(catalogue: Arc<dyn SessionScopeCatalogue>) -> Self {
        Self { catalogue }
    }

    pub async fn execute(
        &self,
        query: &ScopeListQuery,
    ) -> Result<Vec<ScopedSessionRow>, DomainError> {
        self.catalogue.list(query).await
    }
}

impl std::fmt::Debug for ListScopedSessions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListScopedSessions").finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "list_scoped_sessions_tests.rs"]
mod tests;
