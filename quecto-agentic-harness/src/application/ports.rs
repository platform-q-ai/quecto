//! The application-owned ports infrastructure is allowed to depend on.
//!
//! Dependency inversion requires infrastructure adapters to implement
//! application-defined contracts, but nothing more: use cases and query types
//! stay on the application side of the boundary. Routing every such dependency
//! through this one module makes the permitted surface explicit and lets
//! `tests/architecture.rs` enforce it.

pub use super::catalogue::ports::{
    RefreshChange, RefreshContext, RefreshError, RefreshRedactionPort, RefreshableCatalogueSource,
};
pub use super::catalogue::{
    CatalogueSnapshotStore, CatalogueSource, CatalogueSourceError, CredentialStatusPort,
    ResolvedCatalogue, SkippedRecord, SourceEntries,
};
pub use super::provider_runtime::dto::{AdmissionBindingDiagnostic, ProviderRuntimeOutcome};
pub use super::provider_runtime::{ProviderRuntimeFactory, RuntimeSnapshotStore};
pub use crate::application::admission::ports::inference_attempt::{
    AttemptAcquisition, AttemptAdmission, AttemptPermit,
};
pub use crate::application::admission::ports::inference_authority_ports::{
    AdmissionJournal, AdmissionSecretSource, JournalError,
};
pub use crate::application::admission::ports::inference_observation::AdmissionObservation;
pub use crate::application::admission::services::inference_admission::{
    AdmissionClient, AdmissionDispatcher, AdmissionRecovery, AdmissionRegistry,
};
pub use crate::application::admission::services::inference_authority::{
    AdmissionAuthority, AuthorityError, AuthorityStatus, Credential,
};
pub use crate::application::subagent_launch::{LaunchFuture, SubagentLaunchPorts};

#[cfg(any(test, feature = "test-support"))]
pub use super::swarm::LifecycleService as SwarmTestLifecycle;
