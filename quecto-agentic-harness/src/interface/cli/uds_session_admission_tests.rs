use super::uds_session::AgentSession;
#[test]
fn advisory_is_read_from_published_runtime_and_deduplicated() {
    use crate::application::catalogue::{
        CatalogueSnapshotStore, CatalogueSource, CredentialStatusPort, SourceEntries,
    };
    use crate::application::provider_runtime::{
        AdmissionBindingDiagnostic, ComposeProviderRuntimeUseCase, CompositionPorts,
        ProviderRuntimeFactory, ProviderRuntimeOutcome,
    };
    use crate::application::providers::ports::{ChatRequest, LlmProvider};
    use crate::domain::catalogue::{CatalogueEntry, SourceLayer};
    use crate::domain::error::DomainError;
    use crate::domain::message::LlmResponse;
    use std::sync::Arc;
    #[derive(Debug)]
    struct Stub;
    impl LlmProvider for Stub {
        fn name(&self) -> &str {
            "stub"
        }
        fn chat<'a>(
            &'a self,
            _: ChatRequest<'a>,
        ) -> std::pin::Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>>
        {
            Box::pin(async { Err(DomainError::Provider("unused".into())) })
        }
    }
    struct Factory(Vec<String>);
    impl ProviderRuntimeFactory<(), ()> for Factory {
        fn compose_runtime(&self, _: &(), _: &()) -> Result<Arc<dyn LlmProvider>, String> {
            Ok(Arc::new(Stub))
        }
        fn compose_runtime_outcome(
            &self,
            _: &(),
            _: &(),
        ) -> Result<ProviderRuntimeOutcome, String> {
            Ok(ProviderRuntimeOutcome {
                provider: Arc::new(Stub),
                admission_binding_diagnostic: AdmissionBindingDiagnostic {
                    unbound_slots: self.0.clone(),
                },
            })
        }
    }
    struct Source;
    impl CatalogueSource for Source {
        fn id(&self) -> &str {
            "empty"
        }
        fn layer(&self) -> SourceLayer {
            SourceLayer::BuiltIn
        }
        fn load(&self) -> Result<SourceEntries, String> {
            Ok(SourceEntries::from(Vec::<CatalogueEntry>::new()))
        }
    }
    struct Credentials;
    impl CredentialStatusPort for Credentials {
        fn credential_available(&self, _: &CatalogueEntry) -> bool {
            true
        }
    }
    let store = crate::application::ports::RuntimeSnapshotStore::new();
    let catalogue = CatalogueSnapshotStore::empty();
    let mut session = AgentSession::new("stub".into());
    session.observe_runtime(store.clone());
    assert!(session.current_admission_warnings().is_empty());
    let ports = CompositionPorts {
        sources: &[&Source],
        credentials: &Credentials,
        catalogue_store: &catalogue,
        runtime_store: &store,
    };
    ComposeProviderRuntimeUseCase::new()
        .compose_and_publish(
            &Factory(vec!["openai-api".into(), "openai-api".into()]),
            &(),
            &(),
            &ports,
        )
        .unwrap();
    assert_eq!(session.current_admission_warnings().len(), 1);
    assert_eq!(session.current_admission_warnings()[0].slot, "openai-api");
    ComposeProviderRuntimeUseCase::new()
        .compose_and_publish(&Factory(Vec::new()), &(), &(), &ports)
        .unwrap();
    assert!(
        session.current_admission_warnings().is_empty(),
        "reload clears warning"
    );
}
