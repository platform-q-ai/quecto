// Built-in tables, split out of `model_registry.rs` to respect the
// per-file line cap: the built-in models and the auth modes each is offered
// under (#2435), GPT-5.6 tier pricing and windows (#2405), and which
// built-in models take images (#2421). Models older than GPT-5.6 and
// Claude 5 are retired from the tables (#2435); `models.json` can still
// declare one.
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
pub(super) struct Offered {
    pub(super) api_key: bool,
    pub(super) oauth: bool,
}

impl Offered {
    /// Offered under every auth mode of its vendor.
    pub(super) const BOTH: Self = Self {
        api_key: true,
        oauth: true,
    };

    /// Whether a provider authenticating with `auth` offers the model.
    pub(super) fn includes(self, auth: AuthMode) -> bool {
        match auth {
            AuthMode::ApiKey => self.api_key,
            AuthMode::OAuth => self.oauth,
        }
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
    debug_assert!(
        models
            .iter()
            .all(|model| model.offered.api_key || model.offered.oauth),
        "every built-in model is offered under some auth mode"
    );
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
        offered: Offered::BOTH,
    }
}

/// Every built-in model row: each vendor's models under each of its
/// providers that offers them (#2435), then xAI's.
pub(super) fn builtin_specs() -> Vec<BuiltinSpec> {
    const ANTHROPIC: &[BuiltinModel] = &[
        builtin("claude-fable-5-1", "Claude Fable 5.1"),
        builtin("claude-fable-5", "Claude Fable 5"),
        builtin("claude-opus-5", "Claude Opus 5"),
        builtin("claude-sonnet-5", "Claude Sonnet 5"),
    ];
    const OPENAI: &[BuiltinModel] = &[
        builtin("gpt-6-astra", "GPT 6 Astra"),
        builtin("gpt-6-sol", "GPT 6 Sol"),
        builtin("gpt-6.1-sol", "GPT 6.1 Sol"),
        builtin("gpt-6-luna", "GPT 6 Luna"),
        builtin("gpt-5.6-sol", "GPT 5.6 Sol"),
        builtin("gpt-5.6-terra", "GPT 5.6 Terra"),
        builtin("gpt-5.6-luna", "GPT 5.6 Luna"),
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

/// The built-in models that take image input (#2421), by id: every Claude
/// model, every GPT-5.6/GPT-6 tier and every Grok
/// (docs.x.ai/developers/grok-4-7). A built-in not listed takes text only.
const IMAGE_INPUT_IDS: &[&str] = &[
    "claude-fable-5-1",
    "claude-fable-5",
    "claude-opus-5",
    "claude-sonnet-5",
    "gpt-6-astra",
    "gpt-6-sol",
    "gpt-6.1-sol",
    "gpt-6-luna",
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-5.6-luna",
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
