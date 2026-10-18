use super::EffortVocabulary;
use crate::domain::catalogue::value_objects::identity::{ProviderId, TransportKind};
use crate::domain::inference::value_objects::provider::EffortLevel::{self, *};

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
                "claude-opus-5",
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
                "gpt-6-sol",
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
    // A non-reasoning id stays on Chat Completions, where
    // reasoning_effort + tools is rejected: no vocabulary is offered
    // rather than one that 400s.
    assert!(
        levels(
            "openai-api",
            TransportKind::OpenAiCompletions,
            "my-chat-model",
            false
        )
        .is_empty()
    );
}

#[test]
fn xai_grok_vocabularies_follow_the_documented_scales_and_never_offer_none() {
    assert_eq!(
        levels("xai", TransportKind::OpenAiCompletions, "grok-4.7", true),
        vec![Low, Medium, High, XHigh]
    );
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
