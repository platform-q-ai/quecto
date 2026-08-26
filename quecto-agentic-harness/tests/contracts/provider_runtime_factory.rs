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

struct Config {
    marker: &'static str,
}

struct RuntimeInputs {
    client_marker: &'static str,
}

struct Factory;

impl ProviderRuntimeFactory<Config, RuntimeInputs> for Factory {
    fn compose_runtime(
        &self,
        config: &Config,
        runtime_inputs: &RuntimeInputs,
    ) -> Result<Arc<dyn LlmProvider>, String> {
        // The port receives the caller's own config and runtime-input types
        // untouched: the application layer never inspects or clones them.
        assert_eq!(config.marker, "contract");
        assert_eq!(runtime_inputs.client_marker, "test-client");
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
            &Config { marker: "contract" },
            &RuntimeInputs {
                client_marker: "test-client",
            },
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
