use quecto_agentic_harness::application::catalogue::{
    CatalogueQuery, CatalogueSnapshotStore, QueryCatalogueUseCase,
    ResolveModelSelectionUseCase, SelectionFailure,
};
use quecto_agentic_harness::domain::catalogue::*;

fn descriptor(id: &str, availability: Availability) -> ModelDescriptor {
    ModelDescriptor {
        reference: ModelRef::new(ProviderId::new("test").unwrap(), ModelId::new(id).unwrap()),
        display_name: id.into(), transport: TransportKind::OpenAiCompletions,
        auth: AuthIdentity::ApiKey, capabilities: ModelCapabilities::default(), availability,
        base_url: None, auth_header: false, allow_remote_http: false, configured: true,
    }
}

#[test]
fn query_and_selection_share_identity_generation_and_availability() {
    let runnable = descriptor("run", Availability::Runnable);
    let unavailable = descriptor("stop", Availability::KnownButUnavailable { reasons: vec![UnavailableReason::MissingCredential] });
    let store = CatalogueSnapshotStore::new(CatalogueSnapshot::new(7, vec![runnable.clone(), unavailable.clone()]));
    let listed = QueryCatalogueUseCase::new(store.clone()).query(CatalogueQuery::All);
    assert_eq!(listed.generation, 7);
    assert_eq!(ResolveModelSelectionUseCase::new(store.clone()).resolve(&runnable.reference).unwrap().reference, runnable.reference);
    assert!(matches!(ResolveModelSelectionUseCase::new(store).resolve(&unavailable.reference), Err(SelectionFailure::Unavailable { .. })));
}
