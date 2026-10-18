//! Provider and model descriptors and the catalogue entry pairing them.

use crate::domain::catalogue::value_objects::availability::Availability;
use crate::domain::catalogue::value_objects::identity::{
    AuthIdentity, ModelRef, ProviderId, TransportKind,
};
use crate::domain::catalogue::value_objects::model_facts::ModelCapabilities;

/// Display metadata plus the transport and authentication identity of a
/// provider. Two descriptors with the same vendor metadata but different
/// [`AuthIdentity`] values are distinct provider identities.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProviderDescriptor {
    pub id: ProviderId,
    pub display_name: Option<String>,
    pub transport: TransportKind,
    pub auth: AuthIdentity,
}

impl ProviderDescriptor {
    /// Whether two descriptors name the same provider identity (id + auth).
    pub fn same_identity(&self, other: &Self) -> bool {
        self.id == other.id && self.auth == other.auth
    }
}

/// A model's descriptor within the catalogue.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelDescriptor {
    pub reference: ModelRef,
    pub display_name: Option<String>,
    pub capabilities: ModelCapabilities,
    pub availability: Availability,
}

/// One catalogue entry: a model together with the provider identity it runs
/// under. `model.reference.provider()` must equal `provider.id` for the entry
/// to be valid.
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogueEntry {
    pub provider: ProviderDescriptor,
    pub model: ModelDescriptor,
}

impl CatalogueEntry {
    pub fn reference(&self) -> &ModelRef {
        &self.model.reference
    }
}
