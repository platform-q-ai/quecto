use super::*;

fn descriptor(provider: &str, model: &str, display: &str) -> ModelDescriptor {
    ModelDescriptor {
        reference: ModelRef::parse(provider, model).unwrap(),
        display_name: Some(display.to_string()),
        transport: TransportKind::OpenAiCompletions,
        auth: AuthIdentity::ApiKey,
        base_url: Some("https://example.test/v1".to_string()),
        auth_header: true,
        allow_remote_http: false,
        configured: true,
        capabilities: ModelCapabilities {
            input: vec!["text".to_string()],
            context_window: 128_000,
            max_tokens: 4096,
            context_window_explicit: false,
            max_tokens_explicit: false,
            reasoning: false,
            cost: ModelCost::default(),
        },
        availability: Availability::Runnable,
    }
}

#[test]
fn typed_ids_reject_blank_values_and_preserve_stable_serialization() {
    assert!(ProviderId::new("   ").is_err());
    assert!(ModelId::new("").is_err());

    let reference = ModelRef::parse("openai-api", "gpt-5").unwrap();

    assert_eq!(reference.provider().as_str(), "openai-api");
    assert_eq!(reference.model().as_str(), "gpt-5");
    assert_eq!(reference.qualified_id(), "openai-api/gpt-5");
    assert_eq!(
        ModelRef::parse_qualified("openai-api/gpt-5").unwrap(),
        reference
    );
    assert!(ModelRef::parse_qualified("bare-model").is_err());
}

#[test]
fn merge_layers_uses_stable_identity_and_later_precedence_without_reordering() {
    let builtin = vec![
        descriptor("openai-api", "gpt-5", "Builtin GPT"),
        descriptor("anthropic-api", "claude", "Builtin Claude"),
    ];
    let user = vec![
        descriptor("openai-api", "gpt-5", "User GPT"),
        descriptor("custom", "local", "Custom Local"),
    ];

    let snapshot = CatalogueSnapshot::merge_layers(7, vec![builtin, user]);

    assert_eq!(snapshot.generation, 7);
    assert_eq!(snapshot.models().len(), 3);
    assert_eq!(snapshot.models()[0].qualified_id(), "openai-api/gpt-5");
    assert_eq!(
        snapshot.models()[0].display_name.as_deref(),
        Some("User GPT")
    );
    assert_eq!(snapshot.models()[1].qualified_id(), "anthropic-api/claude");
    assert_eq!(snapshot.models()[2].qualified_id(), "custom/local");
}

#[test]
fn availability_keeps_structured_unavailable_reasons() {
    let unavailable = Availability::KnownButUnavailable {
        reasons: vec![UnavailableReason::MissingCredential],
    };

    assert!(!unavailable.runnable());
    assert_eq!(
        unavailable.reasons(),
        &[UnavailableReason::MissingCredential]
    );
    assert!(Availability::Runnable.runnable());
    assert!(Availability::Runnable.reasons().is_empty());
}
