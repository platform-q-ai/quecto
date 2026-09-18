use super::*;

fn capabilities() -> ModelCapabilities {
    ModelCapabilities {
        effort_levels: Vec::new(),
        input_modalities: vec!["text".to_string()],
        context_window: 128_000,
        max_output_tokens: 4096,
        context_window_explicit: true,
        max_output_tokens_explicit: false,
        reasoning: false,
        cost: ModelCost::default(),
    }
}

fn provider_descriptor(id: &str, auth: AuthIdentity) -> ProviderDescriptor {
    ProviderDescriptor {
        id: ProviderId::new(id).unwrap(),
        display_name: Some(id.to_string()),
        transport: TransportKind::OpenAiCompletions,
        auth,
    }
}

fn entry(provider: &str, model: &str, display: &str) -> CatalogueEntry {
    CatalogueEntry {
        provider: provider_descriptor(provider, AuthIdentity::ApiKey),
        model: ModelDescriptor {
            reference: ModelRef::parse(provider, model).unwrap(),
            display_name: Some(display.to_string()),
            capabilities: capabilities(),
            availability: Availability::runnable(),
        },
    }
}

#[test]
fn typed_ids_reject_blank_values() {
    assert_eq!(
        ProviderId::new("   ").unwrap_err(),
        CatalogueDomainError::EmptyProviderId
    );
    assert_eq!(
        ProviderId::new("").unwrap_err(),
        CatalogueDomainError::EmptyProviderId
    );
    assert_eq!(
        ModelId::new("").unwrap_err(),
        CatalogueDomainError::EmptyModelId
    );
    assert_eq!(
        ModelId::new("   ").unwrap_err(),
        CatalogueDomainError::EmptyModelId
    );
}

#[test]
fn model_ref_round_trips_existing_string_ids() {
    let reference = ModelRef::parse("openai-api", "gpt-5").unwrap();
    assert_eq!(reference.provider().as_str(), "openai-api");
    assert_eq!(reference.model().as_str(), "gpt-5");
    assert_eq!(reference.qualified_id(), "openai-api/gpt-5");
    assert_eq!(
        ModelRef::parse_qualified("openai-api/gpt-5").unwrap(),
        reference
    );
    // Model ids containing '/' (e.g. OpenRouter-style vendor/model) must keep
    // the remainder intact: only the first '/' separates provider from model.
    let nested = ModelRef::parse_qualified("router/vendor/model").unwrap();
    assert_eq!(nested.provider().as_str(), "router");
    assert_eq!(nested.model().as_str(), "vendor/model");
    assert_eq!(nested.qualified_id(), "router/vendor/model");
    assert_eq!(
        ModelRef::parse_qualified("bare-model").unwrap_err(),
        CatalogueDomainError::UnqualifiedModelRef("bare-model".to_string())
    );
    // Slash-at-boundary inputs must reject the blank segment, never build a
    // reference that would round-trip into a corrupt qualified string.
    assert_eq!(
        ModelRef::parse_qualified("/gpt-5").unwrap_err(),
        CatalogueDomainError::EmptyProviderId
    );
    assert_eq!(
        ModelRef::parse_qualified("openai-api/").unwrap_err(),
        CatalogueDomainError::EmptyModelId
    );
    assert_eq!(
        ModelRef::parse_qualified("/").unwrap_err(),
        CatalogueDomainError::EmptyProviderId
    );
}

#[test]
fn auth_identity_exposes_oauth_provider() {
    let oauth = AuthIdentity::OAuth {
        provider: Some(ProviderId::new("anthropic-oauth").unwrap()),
    };
    assert_eq!(
        oauth.oauth_provider().map(ProviderId::as_str),
        Some("anthropic-oauth")
    );
    assert!(AuthIdentity::ApiKey.oauth_provider().is_none());
    // OAuth without a named credential provider is a visible misconfiguration:
    // still an OAuth identity, with no provider to report.
    let anonymous = AuthIdentity::OAuth { provider: None };
    assert!(anonymous.oauth_provider().is_none());
}

#[test]
fn auth_identity_distinguishes_provider_identities_sharing_vendor_metadata() {
    let api_key = provider_descriptor("anthropic-api", AuthIdentity::ApiKey);
    let oauth = provider_descriptor(
        "anthropic-oauth",
        AuthIdentity::OAuth {
            provider: Some(ProviderId::new("anthropic-oauth").unwrap()),
        },
    );
    assert!(!api_key.same_identity(&oauth));
    assert!(api_key.same_identity(&api_key.clone()));
    // Same id, different auth: still distinct identities.
    let same_id_oauth =
        provider_descriptor("anthropic-api", AuthIdentity::OAuth { provider: None });
    assert!(!api_key.same_identity(&same_id_oauth));
}

#[test]
fn availability_states_enforce_reason_invariants() {
    let runnable = Availability::runnable();
    assert!(runnable.is_runnable());
    assert_eq!(runnable.status(), AvailabilityStatus::Runnable);
    assert!(runnable.reasons().is_empty());

    let unavailable = Availability::unavailable(
        AvailabilityStatus::Configured,
        vec![UnavailableReason::MissingCredential],
    )
    .unwrap();
    assert!(!unavailable.is_runnable());
    assert_eq!(unavailable.status(), AvailabilityStatus::Configured);
    assert_eq!(
        unavailable.reasons(),
        &[UnavailableReason::MissingCredential]
    );

    assert_eq!(
        Availability::unavailable(
            AvailabilityStatus::Runnable,
            vec![UnavailableReason::MissingCredential]
        )
        .unwrap_err(),
        CatalogueDomainError::RunnableWithReasons
    );
    assert_eq!(
        Availability::unavailable(AvailabilityStatus::Runnable, vec![]).unwrap_err(),
        CatalogueDomainError::RunnableWithReasons
    );
    assert_eq!(
        Availability::unavailable(AvailabilityStatus::Known, vec![]).unwrap_err(),
        CatalogueDomainError::UnavailableWithoutReason
    );
    assert!(AvailabilityStatus::Known < AvailabilityStatus::Configured);
    assert!(AvailabilityStatus::Configured < AvailabilityStatus::Available);
    assert!(AvailabilityStatus::Available < AvailabilityStatus::Runnable);
}

#[test]
fn validate_entry_rejects_provider_mismatch_and_zero_limits() {
    assert!(validate_entry(&entry("openai-api", "gpt-5", "GPT")).is_ok());

    let mut mismatched = entry("openai-api", "gpt-5", "GPT");
    mismatched.provider = provider_descriptor("other", AuthIdentity::ApiKey);
    assert_eq!(
        validate_entry(&mismatched).unwrap_err(),
        CatalogueDomainError::ProviderMismatch {
            entry_provider: "other".to_string(),
            model_provider: "openai-api".to_string(),
        }
    );

    let mut zero_ctx = entry("openai-api", "gpt-5", "GPT");
    zero_ctx.model.capabilities.context_window = 0;
    assert_eq!(
        validate_entry(&zero_ctx).unwrap_err(),
        CatalogueDomainError::ZeroLimit("context_window".to_string())
    );

    let mut zero_out = entry("openai-api", "gpt-5", "GPT");
    zero_out.model.capabilities.max_output_tokens = 0;
    assert_eq!(
        validate_entry(&zero_out).unwrap_err(),
        CatalogueDomainError::ZeroLimit("max_output_tokens".to_string())
    );

    // The minimal non-zero limits are valid: only zero is rejected.
    let mut minimal = entry("openai-api", "gpt-5", "GPT");
    minimal.model.capabilities.context_window = 1;
    minimal.model.capabilities.max_output_tokens = 1;
    assert!(validate_entry(&minimal).is_ok());
}

#[test]
fn resolve_upserts_by_stable_identity_keeping_position() {
    let builtin = vec![
        entry("openai-api", "gpt-5", "Builtin GPT"),
        entry("anthropic-api", "claude", "Builtin Claude"),
    ];
    let user = vec![
        entry("openai-api", "gpt-5", "User GPT"),
        entry("custom", "local", "Custom Local"),
    ];

    let resolution = resolve_catalogue(
        7,
        vec![
            (SourceLayer::BuiltIn, builtin),
            (SourceLayer::UserOverride, user),
        ],
    );
    let snapshot = &resolution.snapshot;
    assert_eq!(snapshot.generation(), 7);
    assert!(resolution.rejected.is_empty());
    assert_eq!(snapshot.entries().len(), 3);
    assert_eq!(
        snapshot.entries()[0].reference().qualified_id(),
        "openai-api/gpt-5"
    );
    assert_eq!(
        snapshot.entries()[0].model.display_name.as_deref(),
        Some("User GPT")
    );
    assert_eq!(
        snapshot.entries()[1].reference().qualified_id(),
        "anthropic-api/claude"
    );
    assert_eq!(
        snapshot.entries()[2].reference().qualified_id(),
        "custom/local"
    );
    // A projection narrows the entries, never the generation.
    let narrowed = snapshot.filtered(|entry| entry.reference().provider().as_str() != "custom");
    assert_eq!(narrowed.generation(), 7);
    assert_eq!(narrowed.entries().len(), 2);
    assert!(
        narrowed
            .entries()
            .iter()
            .all(|entry| entry.reference().provider().as_str() != "custom")
    );
    let found = snapshot
        .find(&ModelRef::parse("custom", "local").unwrap())
        .unwrap();
    assert_eq!(found.model.display_name.as_deref(), Some("Custom Local"));
    assert!(
        snapshot
            .find(&ModelRef::parse("custom", "missing").unwrap())
            .is_none()
    );
}

#[test]
fn resolve_orders_layers_by_precedence_not_input_order() {
    // Handed highest-precedence first: precedence must still win.
    let resolution = resolve_catalogue(
        1,
        vec![
            (
                SourceLayer::UserDefined,
                vec![entry("openai-api", "gpt-5", "User GPT")],
            ),
            (
                SourceLayer::BuiltIn,
                vec![entry("openai-api", "gpt-5", "Builtin GPT")],
            ),
            (
                SourceLayer::Discovered,
                vec![entry("openai-api", "gpt-5", "Discovered GPT")],
            ),
        ],
    );
    assert_eq!(resolution.snapshot.entries().len(), 1);
    assert_eq!(
        resolution.snapshot.entries()[0]
            .model
            .display_name
            .as_deref(),
        Some("User GPT")
    );
    // Full documented order.
    assert!(SourceLayer::BuiltIn < SourceLayer::Generated);
    assert!(SourceLayer::Generated < SourceLayer::Discovered);
    assert!(SourceLayer::Discovered < SourceLayer::Extension);
    assert!(SourceLayer::Extension < SourceLayer::UserDefined);
    assert!(SourceLayer::UserDefined < SourceLayer::UserOverride);
}

#[test]
fn resolve_rejects_invalid_entries_without_corrupting_the_rest() {
    let mut invalid = entry("openai-api", "bad", "Bad");
    invalid.provider = provider_descriptor("mismatch", AuthIdentity::ApiKey);
    let resolution = resolve_catalogue(
        3,
        vec![(
            SourceLayer::BuiltIn,
            vec![
                entry("openai-api", "gpt-5", "GPT"),
                invalid,
                entry("anthropic-api", "claude", "Claude"),
            ],
        )],
    );
    assert_eq!(resolution.snapshot.entries().len(), 2);
    assert_eq!(
        resolution.snapshot.entries()[0].reference().qualified_id(),
        "openai-api/gpt-5"
    );
    assert_eq!(
        resolution.snapshot.entries()[1].reference().qualified_id(),
        "anthropic-api/claude"
    );
    assert_eq!(resolution.rejected.len(), 1);
    assert_eq!(resolution.rejected[0].layer, SourceLayer::BuiltIn);
    assert_eq!(
        resolution.rejected[0].error,
        CatalogueDomainError::ProviderMismatch {
            entry_provider: "mismatch".to_string(),
            model_provider: "openai-api".to_string(),
        }
    );
}

#[test]
fn resolve_is_deterministic_and_last_writer_wins_within_a_layer() {
    // Multiple distinct keys so the assertions below can actually catch an
    // order-nondeterministic implementation (e.g. draining a HashMap), plus a
    // duplicate key to exercise last-writer-wins within the layer.
    let layer = vec![
        entry("openai-api", "gpt-5", "First"),
        entry("anthropic-api", "claude", "Claude"),
        entry("google-api", "gemini", "Gemini"),
        entry("openai-api", "gpt-5", "Second"),
    ];
    let a = resolve_catalogue(5, vec![(SourceLayer::Generated, layer.clone())]);
    let b = resolve_catalogue(5, vec![(SourceLayer::Generated, layer)]);
    assert_eq!(a, b);
    let qualified: Vec<String> = a
        .snapshot
        .entries()
        .iter()
        .map(|e| e.reference().qualified_id())
        .collect();
    // First-seen positions are preserved deterministically even when a later
    // duplicate replaces an entry's payload.
    assert_eq!(
        qualified,
        vec![
            "openai-api/gpt-5".to_string(),
            "anthropic-api/claude".to_string(),
            "google-api/gemini".to_string(),
        ]
    );
    assert_eq!(
        a.snapshot.entries()[0].model.display_name.as_deref(),
        Some("Second")
    );
}

#[test]
fn empty_snapshot_is_explicit() {
    let empty = CatalogueSnapshot::empty(11);
    assert_eq!(empty.generation(), 11);
    assert!(empty.entries().is_empty());
}

mod effort_vocabulary {
    use super::super::{EffortVocabulary, ProviderId, TransportKind};
    use crate::domain::provider::EffortLevel::{self, *};

    fn levels(
        provider: &str,
        transport: TransportKind,
        model: &str,
        reasoning: bool,
    ) -> Vec<EffortLevel> {
        EffortVocabulary::for_model(
            &ProviderId::new(provider).unwrap(),
            &transport,
            model,
            reasoning,
        )
    }

    #[test]
    fn anthropic_messages_use_the_anthropic_scale_regardless_of_the_flag() {
        for reasoning in [true, false] {
            assert_eq!(
                levels(
                    "anthropic-api",
                    TransportKind::AnthropicMessages,
                    "claude-opus-4-8",
                    reasoning
                ),
                vec![Low, Medium, High, Max]
            );
        }
    }

    #[test]
    fn openai_oauth_always_uses_the_openai_scale_and_openai_api_only_for_reasoning_ids() {
        for reasoning in [true, false] {
            assert_eq!(
                levels(
                    "openai-oauth",
                    TransportKind::OpenAiCompletions,
                    "gpt-5.3-codex",
                    reasoning
                ),
                vec![None, Low, Medium, High, XHigh]
            );
        }
        assert_eq!(
            levels(
                "openai-api",
                TransportKind::OpenAiCompletions,
                "gpt-5.6-sol",
                true
            ),
            vec![None, Low, Medium, High, XHigh]
        );
        // gpt-5.5 stays on Chat Completions, where reasoning_effort + tools is
        // rejected: no vocabulary is offered rather than one that 400s.
        assert!(
            levels(
                "openai-api",
                TransportKind::OpenAiCompletions,
                "gpt-5.5",
                false
            )
            .is_empty()
        );
    }

    #[test]
    fn xai_grok_vocabularies_follow_the_documented_scales_and_never_offer_none() {
        assert_eq!(
            levels("xai", TransportKind::OpenAiCompletions, "grok-4.6", true),
            vec![Low, Medium, High, XHigh]
        );
        assert_eq!(
            levels("xai", TransportKind::OpenAiCompletions, "grok-4.5", true),
            vec![Low, Medium, High]
        );
        assert_eq!(
            levels("xai", TransportKind::OpenAiCompletions, "grok-3-mini", true),
            vec![Low, Medium, High]
        );
        // The flag, not the name, is the affirmation.
        assert!(levels("xai", TransportKind::OpenAiCompletions, "grok-4.6", false).is_empty());
        assert!(
            levels(
                "xai",
                TransportKind::OpenAiCompletions,
                "grok-2-image",
                false
            )
            .is_empty()
        );
    }

    #[test]
    fn other_openai_compatible_endpoints_get_the_common_scale_only_when_reasoning_is_declared() {
        assert_eq!(
            levels(
                "fireworks",
                TransportKind::OpenAiCompletions,
                "accounts/fireworks/models/glm-5p3",
                true
            ),
            vec![Low, Medium, High]
        );
        assert!(
            levels(
                "fireworks",
                TransportKind::OpenAiCompletions,
                "accounts/fireworks/models/glm-5p3",
                false
            )
            .is_empty()
        );
        assert!(
            levels(
                "spark-local",
                TransportKind::OpenAiCompletions,
                "qwen3.6-35b-a3b-int4",
                false
            )
            .is_empty()
        );
    }

    #[test]
    fn transports_without_a_reasoning_adapter_offer_nothing() {
        assert!(levels("google", TransportKind::GoogleGenerativeAi, "gemini", true).is_empty());
        assert!(
            levels(
                "x",
                TransportKind::Unsupported {
                    declared: "grpc".into()
                },
                "m",
                true
            )
            .is_empty()
        );
    }

    #[test]
    fn strings_mirror_the_levels_in_order() {
        assert_eq!(
            EffortVocabulary::strings_for_model(
                &ProviderId::new("xai").unwrap(),
                &TransportKind::OpenAiCompletions,
                "grok-4.5",
                true
            ),
            vec!["low", "medium", "high"]
        );
    }
}
