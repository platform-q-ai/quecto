use super::*;
use crate::domain::catalogue::{AuthIdentity, ModelCapabilities, ModelCost, ModelId, ProviderId};

fn descriptor(provider: &str, model: &str, runnable: bool) -> ModelDescriptor {
    ModelDescriptor {
        reference: ModelRef::parse(provider, model).unwrap(),
        display_name: Some(model.to_string()),
        transport: TransportKind::OpenAiCompletions,
        auth: AuthIdentity::ApiKey,
        base_url: Some("https://example.test/v1".to_string()),
        auth_header: true,
        allow_remote_http: false,
        configured: runnable,
        capabilities: ModelCapabilities {
            input: vec!["text".to_string()],
            context_window: 128_000,
            max_tokens: 4096,
            context_window_explicit: false,
            max_tokens_explicit: false,
            reasoning: false,
            cost: ModelCost::default(),
        },
        availability: if runnable {
            Availability::Runnable
        } else {
            Availability::KnownButUnavailable {
                reasons: vec![UnavailableReason::MissingCredential],
            }
        },
    }
}

struct StaticSource {
    name: &'static str,
    models: Vec<ModelDescriptor>,
}

impl CatalogueSource for StaticSource {
    fn name(&self) -> &str {
        self.name
    }
    fn load(&self) -> Result<Vec<ModelDescriptor>, String> {
        Ok(self.models.clone())
    }
}

struct FailingSource;

impl CatalogueSource for FailingSource {
    fn name(&self) -> &str {
        "failing"
    }
    fn load(&self) -> Result<Vec<ModelDescriptor>, String> {
        Err("boom".to_string())
    }
}

#[test]
fn resolve_catalogue_applies_sources_in_declared_precedence() {
    let builtin = StaticSource {
        name: "builtin",
        models: vec![descriptor("openai-api", "gpt-5", true)],
    };
    let user = StaticSource {
        name: "user",
        models: vec![descriptor("openai-api", "gpt-5", false)],
    };

    let snapshot = ResolveCatalogueUseCase::new(vec![&builtin, &user])
        .resolve(5)
        .unwrap();

    assert_eq!(snapshot.generation, 5);
    let model = snapshot
        .find(&ModelRef::parse("openai-api", "gpt-5").unwrap())
        .unwrap();
    assert!(!model.availability.runnable());
    assert_eq!(
        model.availability.reasons(),
        &[UnavailableReason::MissingCredential]
    );
}

#[test]
fn resolve_catalogue_reports_source_failures_without_partial_snapshot() {
    let err = ResolveCatalogueUseCase::new(vec![&FailingSource])
        .resolve(1)
        .unwrap_err();

    assert_eq!(err, "failing: boom");
}

#[test]
fn query_and_selection_read_only_the_published_snapshot() {
    let initial = CatalogueSnapshot::new(
        1,
        vec![
            descriptor("openai-api", "gpt-5", true),
            descriptor("google", "gemini", false),
        ],
    );
    let store = CatalogueSnapshotStore::new(initial);

    let query = QueryCatalogueUseCase::new(store.clone());
    assert_eq!(query.query(CatalogueQuery::All).models().len(), 2);
    assert_eq!(query.query(CatalogueQuery::Runnable).models().len(), 1);

    let selection = ResolveModelSelectionUseCase::new(store.clone());
    assert!(
        selection
            .resolve(&ModelRef::parse("openai-api", "gpt-5").unwrap())
            .is_ok()
    );
    assert_eq!(
        selection
            .resolve(&ModelRef::parse("google", "gemini").unwrap())
            .unwrap_err(),
        SelectionFailure::Unavailable {
            reasons: vec![UnavailableReason::MissingCredential]
        }
    );
    assert_eq!(
        selection
            .resolve(&ModelRef::parse("missing", "model").unwrap())
            .unwrap_err(),
        SelectionFailure::UnknownModel
    );

    store.publish(CatalogueSnapshot::new(
        2,
        vec![descriptor("google", "gemini", true)],
    ));
    assert_eq!(
        query.query(CatalogueQuery::Runnable).models()[0].qualified_id(),
        "google/gemini"
    );
}

#[test]
fn derive_availability_keeps_transport_and_credentials_as_derived_runtime_status() {
    assert_eq!(
        derive_availability(TransportKind::OpenAiCompletions, true, true),
        Availability::Runnable
    );
    assert_eq!(
        derive_availability(TransportKind::GoogleGenerativeAi, false, false),
        Availability::KnownButUnavailable {
            reasons: vec![
                UnavailableReason::UnsupportedTransport {
                    transport: TransportKind::GoogleGenerativeAi
                },
                UnavailableReason::MissingCredential,
            ]
        }
    );
}

#[test]
fn oauth_auth_identity_preserves_auth_billing_provider_identity() {
    let id = AuthIdentity::OAuth {
        provider: ProviderId::new("openai").unwrap(),
    };

    assert_eq!(id.stable_id(), "oauth");
    assert_eq!(id.oauth_provider().unwrap().as_str(), "openai");
    assert_ne!(ModelId::new("gpt-5").unwrap().as_str(), id.stable_id());
}
