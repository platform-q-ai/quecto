//! Discovery composition (#2010): the metadata-search use case over the
//! loop's one home context, its controller, and the discovery handles that
//! carry it beside the list controller the sessions composition built.
use crate::application::sessions::session_home::SessionHomeContext;
use crate::application::sessions::use_cases::SearchSessionMetadata;
use crate::interface::cli::uds_discovery_handles::SessionDiscoveryHandles;
use crate::interface::uds::sessions::controller::ListSessionsController;
use crate::interface::uds::sessions::search_session_metadata_controller::SearchSessionMetadataController;
use std::sync::Arc;

/// The discovery handles of one loop: the list controller the sessions
/// composition built, and the metadata search over the same `home`.
pub fn discovery_handles(
    list: Arc<ListSessionsController>,
    home: SessionHomeContext,
) -> SessionDiscoveryHandles {
    let search = Arc::new(SearchSessionMetadata::new(home));
    SessionDiscoveryHandles::new(list, Arc::new(SearchSessionMetadataController::new(search)))
}

#[cfg(test)]
#[path = "session_search_tests.rs"]
mod tests;
