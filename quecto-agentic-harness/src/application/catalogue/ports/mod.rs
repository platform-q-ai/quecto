//! Capability-local ports of the catalogue. The source and credential ports
//! predate the target layout and stay in the capability root (`CatalogueSource`,
//! `CredentialStatusPort`); this folder holds the ports the use cases added.

pub mod catalogue_inputs;
pub mod effort_runtime;
pub mod effort_vocabulary;
pub mod model_runtime;
pub mod refresh;
pub mod refresh_inputs;
pub mod runtime_snapshot;

pub use catalogue_inputs::{CatalogueInputsLoader, LoadedCatalogueInputs};
pub use effort_runtime::EffortRuntime;
pub use effort_vocabulary::EffortVocabularySource;
pub use model_runtime::ModelRuntime;
#[cfg(any(test, feature = "test-support"))]
pub use refresh::NoopRedaction;
pub use refresh::{
    RefreshChange, RefreshContext, RefreshError, RefreshRedactionPort, RefreshableCatalogueSource,
};
pub use refresh_inputs::{LoadedRefreshInputs, RefreshInputsLoader};
pub use runtime_snapshot::RuntimeSnapshotSource;
