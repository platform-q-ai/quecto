// Built-in OpenAI tables, split out of `model_registry.rs` to respect the
// per-file line cap: GPT-5.6 tier pricing, the published limits of the
// OpenAI/Codex models the GPT-5.6+ enrichment does not cover (#2405,
// checked 2026-10-01), and which built-in models take images (#2421).
// They return infra types, so they live in the infrastructure layer next
// to the registry rather than in the domain.

use super::ModelCost;

/// USD-per-1M-token cost for built-in OpenAI reasoning tiers enriched with
/// published GPT-5.6+/GPT-6 limits, or `None` otherwise.
/// Cache read 0.10x input (90% discount), cache write 1.25x input (OpenAI
/// GPT-5.6+ caching). See `builtin` for source URLs.
pub(super) fn gpt_5_6_cost(id: &str) -> Option<ModelCost> {
    let (input, output) = match id {
        "gpt-6-astra" => (10.0, 50.0),
        "gpt-6-sol" => (2.0, 10.0),
        "gpt-6.1-sol" => (2.0, 10.0),
        "gpt-6-luna" => (0.1, 0.5),
        "gpt-5.6-sol" => (5.0, 30.0),
        "gpt-5.6-terra" => (2.5, 15.0),
        "gpt-5.6-luna" => (1.0, 6.0),
        _ => return None,
    };
    Some(ModelCost {
        input,
        output,
        // GPT-6.1 Sol publishes $0.10/1M cached input (5% of $2.00);
        // older tiers retain the existing 10% cache-read policy.
        cache_read: if id == "gpt-6.1-sol" {
            0.10
        } else {
            input * 0.10
        },
        cache_write: input * 1.25,
    })
}

/// The GPT-5.6/GPT-6 tiers' window on `provider` (#2405 review M3). Codex
/// (`openai-oauth`) runs them at 272k of input beside the 128k output, a
/// 400k window: `context_window: 272000` for each tier in Codex's own
/// `codex-rs/models-manager/models.json` (openai/codex@b1e72963,
/// 2026-09-29; its `max_context_window` of 872000 is an opt-in, not the
/// default). The API publishes 1,050,000 (see `build_builtin`).
pub(super) fn gpt_5_6_window(provider: &str) -> u32 {
    match provider {
        "openai-oauth" => 400_000,
        _ => 1_050_000,
    }
}

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
///   (a 400K window in Codex; Codex's `models.json` gives 272000 of input,
///   openai/codex@b1e72963); the output cap is the model's 128,000.
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

/// The built-in models that take image input (#2421), by id: every Claude
/// model, every GPT-5/GPT-6 tier but GPT-5.3 Codex Spark (text only:
/// openai.com/index/introducing-gpt-5-3-codex-spark) and every Grok
/// (docs.x.ai/developers/grok-4-7). A built-in not listed takes text only.
const IMAGE_INPUT_IDS: &[&str] = &[
    "claude-fable-5-1",
    "claude-fable-5",
    "claude-opus-5",
    "claude-opus-4-8",
    "claude-opus-4-7",
    "claude-opus-4-6",
    "claude-opus-4-5",
    "claude-sonnet-5",
    "claude-sonnet-4-6",
    "claude-sonnet-4-5",
    "gpt-6-astra",
    "gpt-6-sol",
    "gpt-6.1-sol",
    "gpt-6-luna",
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-5.6-luna",
    "gpt-5.5",
    "gpt-5.5-mini",
    "gpt-5.5-nano",
    "gpt-5.3-codex",
    "gpt-5.2-codex",
    "grok-4.7",
    "grok-4.6",
    "grok-4.5",
];

/// The input a built-in model declares (#2421): text and image for one in
/// [`IMAGE_INPUT_IDS`] under a built-in provider, text only for any other
/// model. Every record starts from it, so a discovered listing or a
/// `models.json` entry that says nothing of its input keeps the built-in
/// one; only an explicit `input` replaces it (review L1).
pub(crate) fn builtin_input(provider: &str, id: &str) -> Vec<String> {
    let built_in = super::ModelRegistry::builtin_specs()
        .iter()
        .any(|spec| spec.0 == provider && spec.1 == id);
    match built_in && IMAGE_INPUT_IDS.contains(&id) {
        true => vec!["text".to_string(), "image".to_string()],
        false => vec!["text".to_string()],
    }
}
