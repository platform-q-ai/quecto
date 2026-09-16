//! Capability-local ports of the catalogue. The source and credential ports
//! predate the target layout and stay in the capability root (`CatalogueSource`,
//! `CredentialStatusPort`); this folder holds the ports the use cases added.

pub mod catalogue_inputs;

pub use catalogue_inputs::{CatalogueInputsLoader, LoadedCatalogueInputs};
