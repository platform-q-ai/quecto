//! Session-home composition (#2009): the derived catalogue over the file
//! store's layout, real Git discovery, and the process's execution
//! directory — or the typed reason it has none. Built once per loop and
//! shared by the list query, the save transaction and resume admission.
use crate::application::sessions::session_home::SessionHomeContext;
use crate::infrastructure::persistence::session_home_catalogue::FileSessionHomeCatalogue;
use crate::infrastructure::persistence::session_store::FileSessionStore;
use crate::infrastructure::workspace::git_scope_discovery::GitScopeDiscovery;
use std::sync::Arc;

/// The home context of one loop over `store`, executing in the current
/// directory. An unreadable current directory is carried as the error it
/// raised: every discovery over it is observably unavailable, never `""`.
pub fn build_session_home(store: Arc<FileSessionStore>) -> SessionHomeContext {
    session_home_in(
        store,
        std::env::current_dir().map_err(|error| error.to_string()),
    )
}

/// The home context over `store` for a loop executing in `execution_dir`.
pub fn session_home_in(
    store: Arc<FileSessionStore>,
    execution_dir: Result<std::path::PathBuf, String>,
) -> SessionHomeContext {
    SessionHomeContext {
        catalogue: Arc::new(FileSessionHomeCatalogue::with_store(store)),
        discovery: Arc::new(GitScopeDiscovery::default()),
        execution_dir,
    }
}

#[cfg(test)]
#[path = "session_home_save_tests.rs"]
mod save_tests;
#[cfg(test)]
#[path = "session_home_tests.rs"]
mod tests;
