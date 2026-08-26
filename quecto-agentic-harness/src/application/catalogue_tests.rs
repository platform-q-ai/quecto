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

#[test]
fn known_configured_available_and_runnable_are_distinct_derived_views() {
    let mut unsupported_transport = descriptor("google", "gemini", false);
    unsupported_transport.configured = true;
    unsupported_transport.transport = TransportKind::GoogleGenerativeAi;
    unsupported_transport.availability = Availability::KnownButUnavailable {
        reasons: vec![UnavailableReason::UnsupportedTransport {
            transport: TransportKind::GoogleGenerativeAi,
        }],
    };
    let mut missing_credential = descriptor("fireworks", "glm", false);
    missing_credential.configured = true;
    let unconfigured = descriptor("anthropic-api", "claude", false);

    let store = CatalogueSnapshotStore::new(CatalogueSnapshot::new(
        4,
        vec![
            descriptor("openai-api", "gpt-5", true),
            missing_credential,
            unsupported_transport,
            unconfigured,
        ],
    ));
    let query = QueryCatalogueUseCase::new(store);

    let ids = |filter| {
        query
            .query(filter)
            .models()
            .iter()
            .map(|model| model.qualified_id())
            .collect::<Vec<_>>()
    };

    assert_eq!(ids(CatalogueQuery::Known).len(), 4);
    assert_eq!(
        ids(CatalogueQuery::Configured),
        ["openai-api/gpt-5", "fireworks/glm", "google/gemini"]
    );
    assert_eq!(
        ids(CatalogueQuery::Available),
        ["openai-api/gpt-5", "fireworks/glm"],
        "an entry with no transport adapter is configured but never available"
    );
    assert_eq!(ids(CatalogueQuery::Runnable), ["openai-api/gpt-5"]);
}

#[test]
fn resolve_sources_applies_layer_precedence_and_reports_skipped_layers() {
    struct Layer(&'static str, Vec<ModelDescriptor>);
    struct Broken;

    impl CatalogueSource for Layer {
        fn id(&self) -> &str {
            self.0
        }
        fn load(&self) -> Result<Vec<ModelDescriptor>, String> {
            Ok(self.1.clone())
        }
    }

    impl CatalogueSource for Broken {
        fn id(&self) -> &str {
            "models.json"
        }
        fn load(&self) -> Result<Vec<ModelDescriptor>, String> {
            Err("failed to parse".to_string())
        }
    }

    let builtin = Layer("builtin", vec![descriptor("openai-api", "gpt-5", false)]);
    let runtime = Layer("runtime", vec![descriptor("openai-api", "gpt-5", true)]);

    let resolved = ResolveCatalogueUseCase.resolve_sources(9, &[&builtin, &Broken, &runtime]);

    assert_eq!(resolved.snapshot.generation, 9);
    assert_eq!(resolved.snapshot.models().len(), 1);
    assert!(
        resolved.snapshot.models()[0].availability.runnable(),
        "the runtime layer must win over built-in metadata for the same identity"
    );
    assert_eq!(
        resolved.skipped,
        vec![CatalogueSourceError {
            source: "models.json".to_string(),
            error: "failed to parse".to_string(),
        }]
    );
}

#[test]
fn a_configured_open_provider_routes_model_ids_the_catalogue_cannot_enumerate() {
    use crate::domain::catalogue::ProviderId;

    let snapshot = CatalogueSnapshot::new(2, vec![descriptor("openai-api", "gpt-5", true)])
        .with_open_providers(vec![ProviderId::new("spark").unwrap()]);
    let selection = ResolveModelSelectionUseCase::new(CatalogueSnapshotStore::new(snapshot));

    let open = ModelRef::parse("spark", "qwen3").unwrap();
    assert_eq!(
        selection.resolve(&open).unwrap(),
        ModelSelection::OpenRoute(open.clone()),
        "an explicitly configured endpoint prefix stays selectable"
    );
    assert_eq!(
        selection
            .resolve(&ModelRef::parse("unconfigured", "model").unwrap())
            .unwrap_err(),
        SelectionFailure::UnknownModel
    );
    assert!(matches!(
        selection
            .resolve(&ModelRef::parse("openai-api", "gpt-5").unwrap())
            .unwrap(),
        ModelSelection::Known(_)
    ));
}

#[test]
fn bare_names_resolve_uniquely_and_report_ambiguity_with_candidates() {
    let snapshot = CatalogueSnapshot::new(
        1,
        vec![
            descriptor("openai-api", "gpt-5", true),
            descriptor("openai-oauth", "gpt-5", true),
            descriptor("fireworks", "glm", true),
        ],
    );

    assert_eq!(
        resolve_model_reference(&snapshot, "glm").unwrap(),
        ModelRef::parse("fireworks", "glm").unwrap()
    );
    assert_eq!(
        resolve_model_reference(&snapshot, "openai-api/gpt-5").unwrap(),
        ModelRef::parse("openai-api", "gpt-5").unwrap()
    );
    assert_eq!(
        resolve_model_reference(&snapshot, "gpt-5").unwrap_err(),
        SelectionFailure::AmbiguousModel {
            candidates: vec![
                "openai-api/gpt-5".to_string(),
                "openai-oauth/gpt-5".to_string()
            ]
        },
        "an ambiguous bare name must name its candidates, not read as unknown"
    );
    assert_eq!(
        resolve_model_reference(&snapshot, "nope").unwrap_err(),
        SelectionFailure::UnknownModel
    );
}

#[test]
fn an_unknown_id_under_a_routable_provider_is_selectable_without_limits() {
    use crate::domain::catalogue::ProviderId;

    // The catalogue enumerates what it knows; the runtime decides what it can
    // route. A model newer than the shipped registry must not be refused when
    // its provider is configured and constructible.
    let snapshot = CatalogueSnapshot::new(3, vec![descriptor("openai-api", "gpt-5", true)])
        .with_open_providers(vec![ProviderId::new("openai-api").unwrap()]);
    let selection = ResolveModelSelectionUseCase::new(CatalogueSnapshotStore::new(snapshot));

    let newer = ModelRef::parse("openai-api", "gpt-5.7").unwrap();
    assert_eq!(
        selection.resolve(&newer).unwrap(),
        ModelSelection::OpenRoute(newer)
    );
    assert_eq!(
        selection
            .resolve(&ModelRef::parse("gemini", "gemini-pro").unwrap())
            .unwrap_err(),
        SelectionFailure::UnknownModel,
        "a prefix the runtime cannot route stays an error"
    );
}

#[test]
fn a_known_but_unavailable_model_is_still_rejected_under_a_routable_provider() {
    use crate::domain::catalogue::ProviderId;

    let snapshot = CatalogueSnapshot::new(3, vec![descriptor("openai-api", "gpt-5", false)])
        .with_open_providers(vec![ProviderId::new("openai-api").unwrap()]);
    let selection = ResolveModelSelectionUseCase::new(CatalogueSnapshotStore::new(snapshot));

    assert_eq!(
        selection
            .resolve(&ModelRef::parse("openai-api", "gpt-5").unwrap())
            .unwrap_err(),
        SelectionFailure::Unavailable {
            reasons: vec![UnavailableReason::MissingCredential]
        },
        "an enumerated model keeps its recorded unavailability"
    );
}

#[test]
fn a_query_projection_keeps_the_snapshot_open_providers() {
    use crate::domain::catalogue::ProviderId;

    let store = CatalogueSnapshotStore::new(
        CatalogueSnapshot::new(
            1,
            vec![
                descriptor("openai-api", "gpt-5", true),
                descriptor("google", "gemini", false),
            ],
        )
        .with_open_providers(vec![ProviderId::new("spark").unwrap()]),
    );

    let runnable = QueryCatalogueUseCase::new(store).query(CatalogueQuery::Runnable);

    assert_eq!(runnable.models().len(), 1);
    assert!(
        runnable.accepts_any_model(&ProviderId::new("spark").unwrap()),
        "narrowing the model list must not drop the runtime's routing"
    );
}
