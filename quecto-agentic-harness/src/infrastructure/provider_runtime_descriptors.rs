//! The catalogue projection of one provider composition.
//!
//! Turning registry records into domain descriptors, and recording for each why
//! the runtime could not serve it, is separate from wiring the providers
//! together; keeping it here also keeps `provider_runtime.rs` within the module
//! size gate.

use std::collections::HashSet;

use super::credentials::{self, registry_model_credential_available};
use crate::infrastructure::catalogue_registry::record_to_descriptor_with_credential;
use crate::infrastructure::config::Config;

/// What the catalogue projection of one composition needs to derive each
/// record's availability.
pub(super) struct DescriptorInputs<'a> {
    pub(super) model_registry: &'a crate::infrastructure::model_registry::ModelRegistry,
    pub(super) credentials: &'a credentials::CredentialSnapshot,
    pub(super) config: &'a Config,
    pub(super) canonical_registry_prefixes: &'a HashSet<String>,
    pub(super) configured_endpoint_prefixes: &'a HashSet<String>,
    pub(super) constructible_registry_prefixes: &'a HashSet<String>,
    /// Names of the providers actually constructed, lowercased. Availability is
    /// reconciled against this rather than predicted, so an entry the runtime
    /// declined to build cannot be published as runnable.
    pub(super) constructed_provider_names: &'a HashSet<String>,
    pub(super) has_openai_api_key: bool,
    pub(super) has_anthropic_api_key: bool,
}

/// Project the registry into domain descriptors, recording for each entry why
/// the runtime could not serve it.
pub(super) fn catalogue_descriptors(
    inputs: &DescriptorInputs<'_>,
) -> Result<Vec<crate::domain::catalogue::ModelDescriptor>, String> {
    let mut runtime_model_descriptors = Vec::new();
    for model in inputs.model_registry.models() {
        let credential_available =
            registry_model_credential_available(model, inputs.credentials, inputs.config)?;
        if let Some(mut descriptor) =
            record_to_descriptor_with_credential(model, Some(credential_available))?
        {
            let canonical_provider = model.provider.to_ascii_lowercase();
            let is_canonical_owner = inputs.canonical_registry_prefixes.contains(&model.provider);
            let has_direct_runtime = is_canonical_owner
                && inputs
                    .constructed_provider_names
                    .contains(&canonical_provider)
                && ((canonical_provider == "openai-api" && inputs.has_openai_api_key)
                    || (canonical_provider == "anthropic-api" && inputs.has_anthropic_api_key)
                    || inputs
                        .configured_endpoint_prefixes
                        .contains(&canonical_provider)
                    || inputs
                        .constructible_registry_prefixes
                        .contains(&canonical_provider));
            if !is_canonical_owner || !has_direct_runtime {
                // Keep the reasons derived from the catalogue entry (an
                // unimplemented transport, a missing credential) and add why the
                // runtime skipped it, so availability stays a complete account.
                let mut reasons = descriptor.availability.reasons().to_vec();
                let skipped = crate::domain::catalogue::UnavailableReason::InvalidConfiguration(
                    "no provider was constructed for this prefix".to_string(),
                );
                if !reasons.contains(&skipped) {
                    reasons.push(skipped);
                }
                descriptor.availability =
                    crate::domain::catalogue::Availability::KnownButUnavailable { reasons };
            }
            runtime_model_descriptors.push(descriptor);
        }
    }
    Ok(runtime_model_descriptors)
}
