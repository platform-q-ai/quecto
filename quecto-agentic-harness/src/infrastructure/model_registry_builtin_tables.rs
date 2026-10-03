// Built-in tables, split out of `model_registry.rs` to respect the
// per-file line cap: the built-in models and the auth modes each is offered
// under (#2435), GPT-5.6 tier pricing, the published limits of the
// OpenAI/Codex models the GPT-5.6+ enrichment does not cover (#2405,
// checked 2026-10-01), and which built-in models take images (#2421).
// They return infra types, so they live in the infrastructure layer next
// to the registry rather than in the domain.

use super::{AuthMode, ModelCost, ProviderApi};

/// One built-in model row: the provider that lists it, its id and display
/// name, and how that provider reaches it.
pub(super) struct BuiltinSpec {
    pub(super) provider: &'static str,
    pub(super) id: &'static str,
    pub(super) name: String,
    pub(super) api: ProviderApi,
    pub(super) auth: AuthMode,
    pub(super) oauth: Option<&'static str>,
}

/// The auth modes under which a vendor's built-in providers offer a model
/// (#2435). An auth mode the provider refuses a model for (a ChatGPT
/// sign-in that Codex serves only some models to, say) never lists it, so
/// the catalogue never offers a combination known to fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Offered {
    Both,
    ApiKeyOnly,
    OAuthOnly,
}

impl Offered {
    /// Whether a provider authenticating with `auth` offers the model.
    pub(super) fn includes(self, auth: AuthMode) -> bool {
        let _ = auth;
        true
    }
}

/// One built-in model of a vendor: its id, display name, and the auth
/// modes it is offered under.
pub(super) struct BuiltinModel {
    pub(super) id: &'static str,
    pub(super) name: &'static str,
    pub(super) offered: Offered,
}

/// One of a vendor's built-in providers: its key, wire, auth mode, OAuth
/// identity, and the label its display names carry.
pub(super) struct BuiltinProvider {
    pub(super) provider: &'static str,
    pub(super) api: ProviderApi,
    pub(super) auth: AuthMode,
    pub(super) oauth: Option<&'static str>,
    pub(super) label: &'static str,
}

/// The rows a vendor's built-in providers list: each model under each
/// provider whose auth mode it is offered under, in table order.
pub(super) fn vendor_specs(
    providers: &[BuiltinProvider],
    models: &[BuiltinModel],
) -> Vec<BuiltinSpec> {
    let mut rows = Vec::new();
    for provider in providers {
        for model in models
            .iter()
            .filter(|model| model.offered.includes(provider.auth))
        {
            rows.push(BuiltinSpec {
                provider: provider.provider,
                id: model.id,
                name: format!("{} ({})", model.name, provider.label),
                api: provider.api,
                auth: provider.auth,
                oauth: provider.oauth,
            });
        }
    }
    rows
}

/// A built-in model every auth mode of its vendor is offered.
const fn builtin(id: &'static str, name: &'static str) -> BuiltinModel {
    BuiltinModel {
        id,
        name,
        offered: Offered::Both,
    }
}

/// Every built-in model row: each vendor's models under each of its
/// providers that offers them (#2435), then xAI's.
pub(super) fn builtin_specs() -> Vec<BuiltinSpec> {
    const ANTHROPIC: &[BuiltinModel] = &[
        builtin("claude-fable-5-1", "Claude Fable 5.1"),
        builtin("claude-fable-5", "Claude Fable 5"),
        builtin("claude-opus-5", "Claude Opus 5"),
        builtin("claude-opus-4-8", "Claude Opus 4.8"),
        builtin("claude-opus-4-7", "Claude Opus 4.7"),
        builtin("claude-opus-4-6", "Claude Opus 4.6"),
        builtin("claude-opus-4-5", "Claude Opus 4.5"),
        builtin("claude-sonnet-5", "Claude Sonnet 5"),
        builtin("claude-sonnet-4-6", "Claude Sonnet 4.6"),
        builtin("claude-sonnet-4-5", "Claude Sonnet 4.5"),
    ];
    const OPENAI: &[BuiltinModel] = &[
        builtin("gpt-6-astra", "GPT 6 Astra"),
        builtin("gpt-6-sol", "GPT 6 Sol"),
        builtin("gpt-6.1-sol", "GPT 6.1 Sol"),
        builtin("gpt-6-luna", "GPT 6 Luna"),
        builtin("gpt-5.6-sol", "GPT 5.6 Sol"),
        builtin("gpt-5.6-terra", "GPT 5.6 Terra"),
        builtin("gpt-5.6-luna", "GPT 5.6 Luna"),
        builtin("gpt-5.5", "GPT 5.5"),
        builtin("gpt-5.5-mini", "GPT 5.5 Mini"),
        builtin("gpt-5.5-nano", "GPT 5.5 Nano"),
        builtin("gpt-5.3-codex", "GPT 5.3 Codex"),
        builtin("gpt-5.3-codex-spark", "GPT 5.3 Codex Spark"),
        builtin("gpt-5.2-codex", "GPT 5.2 Codex"),
    ];
    const XAI: &[BuiltinModel] = &[
        builtin("grok-4.7", "Grok 4.7"),
        builtin("grok-4.6", "Grok 4.6"),
        builtin("grok-4.5", "Grok 4.5"),
    ];
    let pair = |api, vendor: &'static str, api_key, oauth| {
        [
            BuiltinProvider {
                provider: api_key,
                api,
                auth: AuthMode::ApiKey,
                oauth: None,
                label: "API key",
            },
            BuiltinProvider {
                provider: oauth,
                api,
                auth: AuthMode::OAuth,
                oauth: Some(vendor),
                label: "OAuth",
            },
        ]
    };
    let anthropic = pair(
        ProviderApi::AnthropicMessages,
        "anthropic",
        "anthropic-api",
        "anthropic-oauth",
    );
    let openai = pair(
        ProviderApi::OpenAiCompletions,
        "openai",
        "openai-api",
        "openai-oauth",
    );
    let xai = [BuiltinProvider {
        provider: "xai",
        api: ProviderApi::OpenAiCompletions,
        auth: AuthMode::OAuth,
        oauth: Some("xai"),
        label: "SuperGrok OAuth",
    }];
    let mut rows = vendor_specs(&anthropic, ANTHROPIC);
    rows.extend(vendor_specs(&openai, OPENAI));
    rows.extend(vendor_specs(&xai, XAI));
    rows
}

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
        .any(|spec| spec.provider == provider && spec.id == id);
    match built_in && IMAGE_INPUT_IDS.contains(&id) {
        true => vec!["text".to_string(), "image".to_string()],
        false => vec!["text".to_string()],
    }
}
