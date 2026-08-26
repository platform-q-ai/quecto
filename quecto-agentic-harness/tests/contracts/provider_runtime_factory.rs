//! Contract coverage for the provider runtime factory port (issue #1573):
//! infrastructure implements `ProviderRuntimeFactory`; the application compose
//! use case drives it with the caller's config/runtime-input types and
//! publishes the composed runtime paired with the catalogue generation.

use std::sync::Arc;

use quecto::application::catalogue::{
    CatalogueSnapshotStore, CatalogueSource, CredentialStatusPort, SourceEntries,
};
use quecto::application::provider_runtime::{
    ComposeProviderRuntimeUseCase, CompositionPorts, ProviderRuntimeFactory, RuntimeSnapshotStore,
};
use quecto::domain::catalogue::{CatalogueEntry, SourceLayer};
use quecto::domain::message::LlmResponse;
use quecto::domain::provider::{ChatRequest, LlmProvider};

#[derive(Debug)]
struct Provider;

impl LlmProvider for Provider {
    fn name(&self) -> &str {
        "runtime-factory-provider"
    }

    fn chat<'a>(
        &'a self,
        _request: ChatRequest<'a>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<LlmResponse, quecto::domain::error::DomainError>,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async {
            Err(quecto::domain::error::DomainError::Provider(
                "contract".into(),
            ))
        })
    }
}

// Opaque caller-owned types: the use case is generic over them with no
// bounds, so the compiler itself guarantees pass-through — the application
// layer cannot inspect, clone, or substitute them.
struct Config;

struct RuntimeInputs;

struct Factory;

impl ProviderRuntimeFactory<Config, RuntimeInputs> for Factory {
    fn compose_runtime(
        &self,
        config: &Config,
        runtime_inputs: &RuntimeInputs,
    ) -> Result<Arc<dyn LlmProvider>, String> {
        // Pass-through of `config`/`runtime_inputs` is enforced by the
        // unbounded generics on the use case, not asserted here.
        let (_, _) = (config, runtime_inputs);
        Ok(Arc::new(Provider))
    }
}

struct EmptySource;

impl CatalogueSource for EmptySource {
    fn id(&self) -> &str {
        "contract-source"
    }
    fn layer(&self) -> SourceLayer {
        SourceLayer::BuiltIn
    }
    fn load(&self) -> Result<SourceEntries, String> {
        Ok(SourceEntries::default())
    }
}

struct AllowAll;

impl CredentialStatusPort for AllowAll {
    fn credential_available(&self, _entry: &CatalogueEntry) -> bool {
        true
    }
}

#[test]
fn provider_runtime_factory_composes_through_application_use_case() {
    let catalogue_store = CatalogueSnapshotStore::empty();
    let runtime_store = RuntimeSnapshotStore::new();
    let composed = ComposeProviderRuntimeUseCase::new()
        .compose_and_publish(
            &Factory,
            &Config,
            &RuntimeInputs,
            &CompositionPorts {
                sources: &[&EmptySource],
                credentials: &AllowAll,
                catalogue_store: &catalogue_store,
                runtime_store: &runtime_store,
            },
        )
        .unwrap();
    assert_eq!(
        composed.snapshot.provider.name(),
        "runtime-factory-provider"
    );
    // Publish pairs the runtime with the catalogue generation it resolved.
    assert_eq!(
        composed.snapshot.generation(),
        catalogue_store.current().generation()
    );
    assert!(runtime_store.current().is_some());
}
