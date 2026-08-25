//! The application-owned ports infrastructure is allowed to depend on.
//!
//! Dependency inversion requires infrastructure adapters to implement
//! application-defined contracts, but nothing more: use cases, snapshot stores,
//! and query types stay on the application side of the boundary. Routing every
//! such dependency through this one module makes the permitted surface explicit
//! and lets `tests/architecture.rs` enforce it.

pub use super::catalogue::{CatalogueSource, derive_availability};
pub use super::catalogue_refresh::{
    CatalogueRefreshAllPort, CatalogueRefreshOutcome, CatalogueRefreshPort, CatalogueRefreshStatus,
};
pub use super::provider_runtime::ProviderRuntimeFactory;
