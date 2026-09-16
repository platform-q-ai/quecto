//! The inputs one refresh run needs, loaded as one on-disk state (#1846):
//! the refreshable sources configured in `models.json`, the full
//! precedence-ordered source set to resolve afterwards, credential status,
//! and the redaction of the secrets those sources carry. Enumerating them is
//! infrastructure's; the use case only drives what it is handed.

use crate::application::catalogue::ports::{RefreshRedactionPort, RefreshableCatalogueSource};
use crate::application::catalogue::{CatalogueSource, CredentialStatusPort};

/// One loaded set of refresh inputs. Sources are borrowed from the load so a
/// run's enumeration and its post-refresh resolve describe one read.
pub trait LoadedRefreshInputs: Send + Sync {
    /// Sources that can be refreshed remotely, in configured order.
    fn refreshables(&self) -> Vec<&dyn RefreshableCatalogueSource>;
    /// The full source set to resolve after refreshing: every layer, with
    /// the refreshables' persisted data feeding the discovered layer.
    fn sources(&self) -> Vec<&dyn CatalogueSource>;
    fn credentials(&self) -> &dyn CredentialStatusPort;
    fn redaction(&self) -> &dyn RefreshRedactionPort;
}

/// Loads the refresh inputs afresh for every run. A catalogue file that
/// cannot be enumerated is an error naming why, never a partial set.
pub trait RefreshInputsLoader: Send + Sync {
    fn load(&self) -> Result<Box<dyn LoadedRefreshInputs>, String>;
}
