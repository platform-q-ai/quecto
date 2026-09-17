//! Provider-runtime composition (#1849 PR 1; formerly the interface's
//! `catalogue_runtime.rs` and `cli/agent_provider.rs`, epic #1193 slice 3):
//! the one place that sees both the compose-provider-runtime use case and
//! the infrastructure adapters it runs over. `main` hands
//! [`build_agent_provider`] to the CLI for startup, and the reload use case
//! composed in `composition::catalogue` (#1849 PR 2) rebuilds through it;
//! the interface never constructs provider state itself.

use std::path::Path;
use std::sync::Arc;

use crate::application::provider_runtime::{
    CatalogueRuntimeSnapshot, ComposeProviderRuntimeUseCase, CompositionPorts,
    RuntimeCompositionError,
};
use crate::application::providers::ports::LlmProvider;
use crate::infrastructure::catalogue_inputs::CatalogueInputs;
use crate::infrastructure::catalogue_registry::{runtime_store_for, snapshot_store_for};
use crate::infrastructure::config::Config;
use crate::infrastructure::provider_runtime::{AgentProviderRuntimeFactory, AgentRuntimeInputs};
use crate::infrastructure::provider_runtime_admission::{
    AdmissionProviderRuntimeFactory, AdmissionRuntimeCandidate,
};
use crate::infrastructure::providers::refresh_wiring::{
    make_oauth_refresh_fn, make_provider_factory,
};

/// Compose the concrete provider runtime for `base_dir` via the shared use
/// case and publish runtime + catalogue as one coherent generation into the
/// process-wide stores for that directory. A failed composition retains the
/// previously published generation (echoed in the error).
pub fn compose_and_publish_runtime(
    config: &Config,
    base_dir: &Path,
    http_client: &reqwest::Client,
) -> Result<Arc<CatalogueRuntimeSnapshot>, RuntimeCompositionError> {
    let catalogue_inputs = CatalogueInputs::load(base_dir);
    let inputs = AgentRuntimeInputs {
        base_dir: base_dir.to_path_buf(),
        http_client: http_client.clone(),
        refresh_fn: make_oauth_refresh_fn(),
        openai_oauth_factory: make_provider_factory(
            "openai",
            openai_api_base(config),
            http_client.clone(),
        ),
        // Same on-disk read as the catalogue sources above: one compose never
        // pairs a catalogue and a router built from different models.json
        // states (and models.json is parsed once per compose, not twice).
        model_registry: catalogue_inputs.effective_registry(),
    };
    let catalogue_store = snapshot_store_for(base_dir);
    let runtime_store = runtime_store_for(base_dir);
    let ports = CompositionPorts {
        sources: &catalogue_inputs.sources(),
        credentials: &catalogue_inputs.credentials,
        catalogue_store: &catalogue_store,
        runtime_store: &runtime_store,
    };
    // An installed admission binding makes every composition (startup and
    // reload) go through the restart-only admission factory: a changed policy
    // or binding set is rejected instead of replacing live budgets (#1679).
    let composed = match crate::infrastructure::admission::process::current() {
        Some(admission) => {
            let proposal = admission_candidate(config);
            ComposeProviderRuntimeUseCase::new().compose_and_publish(
                &AdmissionProviderRuntimeFactory::new(admission.runtime_context().clone()),
                &AdmissionRuntimeCandidate {
                    providers: config,
                    admission: proposal.as_ref(),
                },
                &inputs,
                &ports,
            )?
        }
        None => ComposeProviderRuntimeUseCase::new().compose_and_publish(
            &AgentProviderRuntimeFactory,
            config,
            &inputs,
            &ports,
        )?,
    };
    Ok(composed.snapshot)
}

/// The configured proposal offered to the restart-only admission factory. An
/// invalid or removed section yields `None`, which the factory rejects as a
/// change rather than replacing live budgets.
pub fn admission_candidate(
    config: &Config,
) -> Option<crate::infrastructure::provider_runtime_admission::AdmissionRuntimeProposal> {
    config
        .admission_proposal()
        .ok()
        .flatten()
        .map(|(_, proposal)| proposal)
}

/// Delegates to the factory's own blank/trim helper so the initially composed
/// provider and the post-refresh rebuilt provider share one base-URL reading.
fn openai_api_base(config: &Config) -> Option<String> {
    crate::infrastructure::provider_runtime::non_empty(config.providers.openai.api_base.clone())
}

/// Compose and publish the provider runtime for this base directory, then
/// return its routing provider. The published runtime and catalogue always
/// describe one coherent generation; a failed composition retains the
/// previously published one.
pub fn build_agent_provider(
    config: &Config,
    base_dir: &Path,
    http_client: &reqwest::Client,
) -> Result<Arc<dyn LlmProvider>, String> {
    compose_and_publish_runtime(config, base_dir, http_client)
        .map(|snapshot| snapshot.provider.clone())
        .map_err(|error| error.error)
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "runtime_1066_tests.rs"]
mod provider_1066_tests;
