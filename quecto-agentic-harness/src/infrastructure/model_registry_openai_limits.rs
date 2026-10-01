// Published limits for the built-in OpenAI/Codex models the GPT-5.6+
// enrichment does not cover (#2405), split out of `model_registry.rs` to
// respect the per-file line cap. Checked 2026-10-01.

/// A model's published limits: its context window (input and output
/// together) and, when the provider publishes one, its output cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PublishedLimits {
    pub(super) context_window: u32,
    pub(super) max_output_tokens: Option<u32>,
}

const fn limits(context_window: u32, max_output_tokens: Option<u32>) -> PublishedLimits {
    PublishedLimits {
        context_window,
        max_output_tokens,
    }
}

/// The published limits of a built-in OpenAI/Codex model listed under
/// `provider`, or `None` when no value could be confirmed: `gpt-5.5-mini`
/// and `gpt-5.5-nano` have no model page, so they keep the configured
/// budget. Sources:
/// - gpt-5.5 (API): developers.openai.com/api/docs/models/gpt-5.5
///   (1,050,000 window, 128,000 output).
/// - gpt-5.5 in Codex (ChatGPT sign-in): openai.com/index/introducing-gpt-5-5
///   (a 400K window in Codex); the output cap is the model's 128,000.
/// - gpt-5.3-codex, gpt-5.2-codex: developers.openai.com/api/docs/models/{id}
///   (400,000 window, 128,000 output).
/// - gpt-5.3-codex-spark: openai.com/index/introducing-gpt-5-3-codex-spark
///   (a 128k window; no published output cap).
pub(super) fn openai_published_limits(provider: &str, id: &str) -> Option<PublishedLimits> {
    match (provider, id) {
        ("openai-api", "gpt-5.5") => Some(limits(1_050_000, Some(128_000))),
        ("openai-oauth", "gpt-5.5") => Some(limits(400_000, Some(128_000))),
        ("openai-api" | "openai-oauth", "gpt-5.3-codex" | "gpt-5.2-codex") => {
            Some(limits(400_000, Some(128_000)))
        }
        ("openai-api" | "openai-oauth", "gpt-5.3-codex-spark") => Some(limits(128_000, None)),
        _ => None,
    }
}
