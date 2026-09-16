//! The one effect the read use cases need: load the catalogue inputs of a
//! run — its ordered sources and its credential status — as one on-disk
//! state (#1845). Reading `models.json`, discovery caches and the credential
//! store is infrastructure's; the use case only resolves what it is handed.

use crate::application::catalogue::{CatalogueSource, CredentialStatusPort};

/// One loaded set of inputs. Sources are borrowed from the load so a
/// resolve describes exactly one read.
pub trait LoadedCatalogueInputs: Send + Sync {
    /// The sources in application precedence order (lowest layer first).
    fn sources(&self) -> Vec<&dyn CatalogueSource>;
    fn credentials(&self) -> &dyn CredentialStatusPort;
}

/// Loads the inputs afresh on every call, so a listing stays level with
/// on-disk edits (the pre-refresh contract of `list_models`). Touches no
/// network by contract.
pub trait CatalogueInputsLoader: Send + Sync {
    fn load(&self) -> Box<dyn LoadedCatalogueInputs>;
}
