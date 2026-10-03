//! The reasoning-effort vocabulary rule (#1996), split out of
//! `catalogue.rs` to respect the per-file line cap.

use super::{ProviderId, TransportKind};
use crate::domain::provider::EffortLevel;

/// The reasoning-effort vocabulary a model accepts on the wire (#1996): the
/// one domain rule that seeds [`ModelCapabilities::effort_levels`]. Every
/// surface — the `get_state` listing, the selector, `set_effort`, spawn and
/// startup validation, the provider adapters — consumes the seeded field;
/// none re-derives a vocabulary from a name.
///
/// The rule is affirmative per provider and model, from the providers'
/// documented scales:
///
/// - Anthropic Messages: `low, medium, high, max` (`output_config.effort`).
/// - `openai-oauth` (the Codex Responses API, which transmits a configured
///   `reasoning.effort`; its Chat-Completions fallback for a token without
///   an account id never transmits one): the OpenAI scale
///   `none, low, medium, high, xhigh`.
/// - `openai-api`: the OpenAI scale **only for records declaring
///   `reasoning`** — those are the ids the endpoint router sends to the
///   Responses API; a non-reasoning id stays on Chat Completions, which
///   rejects `reasoning_effort` with function tools, so it offers nothing.
/// - xAI Grok models declaring reasoning: `grok-4.6` and `grok-4.7` = `low,
///   medium, high, xhigh`; every other = `low, medium, high`. Reasoning cannot be
///   disabled, so `none` is never offered.
/// - Any other OpenAI-compatible endpoint (Fireworks, local servers, custom
///   providers): the common `low, medium, high` **only when the record
///   declares `reasoning: true`**; otherwise nothing, so no reasoning option
///   is ever sent to an endpoint that never claimed to accept one.
/// - Transports with no reasoning-option adapter: nothing.
pub struct EffortVocabulary;

impl EffortVocabulary {
    const ANTHROPIC: &'static [EffortLevel] = &[
        EffortLevel::Low,
        EffortLevel::Medium,
        EffortLevel::High,
        EffortLevel::Max,
    ];
    const OPENAI: &'static [EffortLevel] = &[
        EffortLevel::None,
        EffortLevel::Low,
        EffortLevel::Medium,
        EffortLevel::High,
        EffortLevel::XHigh,
    ];
    const COMMON: &'static [EffortLevel] =
        &[EffortLevel::Low, EffortLevel::Medium, EffortLevel::High];
    const XAI_GROK_4_6: &'static [EffortLevel] = &[
        EffortLevel::Low,
        EffortLevel::Medium,
        EffortLevel::High,
        EffortLevel::XHigh,
    ];

    /// The ordered vocabulary for `model_id` served by `provider` over
    /// `transport`, given whether the record declares reasoning.
    pub fn for_model(
        provider: &ProviderId,
        transport: &TransportKind,
        model_id: &str,
        reasoning: bool,
    ) -> Vec<EffortLevel> {
        let levels: &[EffortLevel] = match transport {
            TransportKind::AnthropicMessages => Self::ANTHROPIC,
            TransportKind::OpenAiCompletions => match provider.as_str() {
                "openai-oauth" => Self::OPENAI,
                "openai-api" if reasoning => Self::OPENAI,
                "xai"
                    if reasoning
                        && ["grok-4.6", "grok-4.7"]
                            .iter()
                            .any(|prefix| model_id.starts_with(prefix)) =>
                {
                    Self::XAI_GROK_4_6
                }
                _ if reasoning => Self::COMMON,
                _ => &[],
            },
            TransportKind::GoogleGenerativeAi | TransportKind::Unsupported { .. } => &[],
        };
        levels.to_vec()
    }

    /// The same rule as API strings, the shape `effort_levels` stores.
    pub fn strings_for_model(
        provider: &ProviderId,
        transport: &TransportKind,
        model_id: &str,
        reasoning: bool,
    ) -> Vec<String> {
        Self::for_model(provider, transport, model_id, reasoning)
            .into_iter()
            .map(|level| level.as_str().to_string())
            .collect()
    }
}
