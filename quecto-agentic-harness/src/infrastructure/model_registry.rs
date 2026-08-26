use serde::Deserialize;

use crate::domain::message::claude_sonnet_5_pricing;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct ModelRegistry {
    models: Vec<ModelRecord>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelRecord {
    pub provider: String,
    pub id: String,
    pub display_name: Option<String>,
    pub api: ProviderApi,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub auth_header: bool,
    pub allow_remote_http: bool,
    pub input: Vec<String>,
    pub context_window: u32,
    pub max_tokens: u32,
    /// Whether `max_tokens` was explicitly declared for this model (JSON
    /// `maxTokens`) rather than the synthesized default. Only an explicit cap is
    /// a real output limit, so the #935 clamp (`max_tokens_for`) applies only
    /// when this is true — a listed model that omits `maxTokens` must not be
    /// silently clamped to the default.
    pub max_tokens_explicit: bool,
    /// Whether `context_window` was explicitly declared for this model rather
    /// than the synthesized default. Only an explicit window is a real model
    /// limit, so the #1044 window-aware budget (`context_window_for`) applies
    /// only when this is true.
    pub context_window_explicit: bool,
    pub cost: ModelCost,
    pub reasoning: bool,
    /// How this provider authenticates. `ApiKey` uses the resolved `api_key`;
    /// `OAuth` resolves a credential from the kernel credential store keyed by
    /// `oauth_provider`.
    pub auth: AuthMode,
    /// For `AuthMode::OAuth`, the kernel-known OAuth provider identity to resolve
    /// the credential against (e.g. "anthropic", "openai"). `None` for ApiKey.
    pub oauth_provider: Option<String>,
}

/// Authentication mode for a registry provider.
///
/// Explicit and orthogonal to the wire protocol (`ProviderApi`): the same
/// vendor can be configured twice (once per mode) under distinct provider keys,
/// and the kernel never silently switches between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthMode {
    /// Authenticate with a literal/resolved API key (token billing).
    #[default]
    ApiKey,
    /// Authenticate with an OAuth credential from the kernel credential store.
    OAuth,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModelCost {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderApi {
    OpenAiCompletions,
    AnthropicMessages,
    GoogleGenerativeAi,
}

impl ProviderApi {
    fn parse(value: &str) -> Result<Self, ModelRegistryError> {
        match value {
            "openai-completions" => Ok(Self::OpenAiCompletions),
            "anthropic-messages" => Ok(Self::AnthropicMessages),
            "google-generative-ai" => Ok(Self::GoogleGenerativeAi),
            other => Err(ModelRegistryError::UnknownApi(other.to_string())),
        }
    }
}

#[derive(Debug)]
pub enum ModelRegistryError {
    Io(std::io::Error),
    Parse(serde_json::Error),
    UnknownApi(String),
    UnknownAuthMode(String),
}

impl std::fmt::Display for ModelRegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "failed to read models registry: {e}"),
            Self::Parse(e) => write!(f, "failed to parse models registry: {e}"),
            Self::UnknownApi(api) => write!(f, "unknown api '{api}' in models registry"),
            Self::UnknownAuthMode(mode) => {
                write!(f, "unknown auth mode '{mode}' in models registry")
            }
        }
    }
}

impl std::error::Error for ModelRegistryError {}

type BuiltinSpec = (
    &'static str,
    &'static str,
    &'static str,
    ProviderApi,
    AuthMode,
    Option<&'static str>,
);

impl ModelRegistry {
    /// The built-in model table, constructed once and shared. The ~30 records
    /// are identical on every call, so we build them a single time behind a
    /// `OnceLock` and hand out cheap clones instead of rebuilding at each of
    /// the several call sites.
    pub fn builtin() -> Self {
        use std::sync::OnceLock;
        static BUILTIN: OnceLock<ModelRegistry> = OnceLock::new();
        BUILTIN.get_or_init(Self::build_builtin).clone()
    }

    fn build_builtin() -> Self {
        let mut r = Self { models: Vec::new() };
        for (provider, id, name, api, auth, oauth_provider) in Self::builtin_specs() {
            let mut record = ModelRecord::with_defaults(provider, id, Some(name), api);
            record.auth = auth;
            record.oauth_provider = oauth_provider.map(str::to_string);
            if id == "claude-sonnet-5" {
                record.input = vec!["text".to_string(), "image".to_string()];
                record.context_window = 1_000_000;
                record.context_window_explicit = true;
                record.max_tokens = 128_000;
                record.max_tokens_explicit = true;
                let pricing = claude_sonnet_5_pricing();
                record.cost = ModelCost {
                    input: pricing.input_micro_usd_per_million as f64 / 1_000_000.0,
                    output: pricing.output_micro_usd_per_million as f64 / 1_000_000.0,
                    cache_read: pricing.cache_read_micro_usd_per_million as f64 / 1_000_000.0,
                    cache_write: pricing.cache_write_micro_usd_per_million as f64 / 1_000_000.0,
                };
            } else if let Some(cost) = gpt_5_6_cost(id) {
                // GPT-5.6 tiers share these published limits (2026-07-09);
                // sources: openai.com/index/previewing-gpt-5-6-sol and
                // developers.openai.com/api/docs/models/gpt-5.6-{sol,terra,luna}.
                record.context_window = 1_050_000;
                record.context_window_explicit = true;
                record.max_tokens = 128_000;
                record.max_tokens_explicit = true;
                record.reasoning = true;
                record.cost = cost;
            } else if id == "grok-4.6" {
                // xAI published specs: 500K context, $2/M in, $6/M out,
                // $0.50/M cached input (docs.x.ai/developers/models/grok-4.6).
                record.input = vec!["text".to_string(), "image".to_string()];
                record.context_window = 500_000;
                record.context_window_explicit = true;
                record.reasoning = true;
                record.cost = ModelCost {
                    input: 2.0,
                    output: 6.0,
                    cache_read: 0.5,
                    cache_write: 0.0,
                };
            } else if id == "grok-4.5" {
                // xAI published specs: 500K context, $2/M in, $6/M out
                // (docs.x.ai/developers/grok-4-5).
                record.input = vec!["text".to_string(), "image".to_string()];
                record.context_window = 500_000;
                record.context_window_explicit = true;
                record.reasoning = true;
                record.cost = ModelCost {
                    input: 2.0,
                    output: 6.0,
                    cache_read: 0.0,
                    cache_write: 0.0,
                };
            }
            r.upsert(record);
        }
        r
    }

    /// The (provider, id, display_name, api, auth, oauth) rows for every
    /// built-in model, grouped by their shared provider/auth so each model is
    /// a single (id, name) line.
    fn builtin_specs() -> Vec<BuiltinSpec> {
        let mut v: Vec<BuiltinSpec> = Vec::new();
        let mut group = |provider,
                         api,
                         auth,
                         oauth: Option<&'static str>,
                         ids: &[(&'static str, &'static str)]| {
            for &(id, name) in ids {
                v.push((provider, id, name, api, auth, oauth));
            }
        };
        group(
            "anthropic-api",
            ProviderApi::AnthropicMessages,
            AuthMode::ApiKey,
            None,
            &[
                ("claude-fable-5", "Claude Fable 5 (API key)"),
                ("claude-opus-5", "Claude Opus 5 (API key)"),
                ("claude-opus-4-8", "Claude Opus 4.8 (API key)"),
                ("claude-opus-4-7", "Claude Opus 4.7 (API key)"),
                ("claude-opus-4-6", "Claude Opus 4.6 (API key)"),
                ("claude-opus-4-5", "Claude Opus 4.5 (API key)"),
                ("claude-sonnet-5", "Claude Sonnet 5 (API key)"),
                ("claude-sonnet-4-6", "Claude Sonnet 4.6 (API key)"),
                ("claude-sonnet-4-5", "Claude Sonnet 4.5 (API key)"),
            ],
        );
        group(
            "anthropic-oauth",
            ProviderApi::AnthropicMessages,
            AuthMode::OAuth,
            Some("anthropic"),
            &[
                ("claude-fable-5", "Claude Fable 5 (OAuth)"),
                ("claude-opus-5", "Claude Opus 5 (OAuth)"),
                ("claude-opus-4-8", "Claude Opus 4.8 (OAuth)"),
                ("claude-opus-4-7", "Claude Opus 4.7 (OAuth)"),
                ("claude-opus-4-6", "Claude Opus 4.6 (OAuth)"),
                ("claude-opus-4-5", "Claude Opus 4.5 (OAuth)"),
                ("claude-sonnet-5", "Claude Sonnet 5 (OAuth)"),
                ("claude-sonnet-4-6", "Claude Sonnet 4.6 (OAuth)"),
                ("claude-sonnet-4-5", "Claude Sonnet 4.5 (OAuth)"),
            ],
        );
        group(
            "openai-api",
            ProviderApi::OpenAiCompletions,
            AuthMode::ApiKey,
            None,
            &[
                ("gpt-5.6-sol", "GPT 5.6 Sol (API key)"),
                ("gpt-5.6-terra", "GPT 5.6 Terra (API key)"),
                ("gpt-5.6-luna", "GPT 5.6 Luna (API key)"),
                ("gpt-5.5", "GPT 5.5 (API key)"),
                ("gpt-5.5-mini", "GPT 5.5 Mini (API key)"),
                ("gpt-5.5-nano", "GPT 5.5 Nano (API key)"),
                ("gpt-5.3-codex", "GPT 5.3 Codex (API key)"),
                ("gpt-5.3-codex-spark", "GPT 5.3 Codex Spark (API key)"),
                ("gpt-5.2-codex", "GPT 5.2 Codex (API key)"),
            ],
        );
        group(
            "openai-oauth",
            ProviderApi::OpenAiCompletions,
            AuthMode::OAuth,
            Some("openai"),
            &[
                ("gpt-5.6-sol", "GPT 5.6 Sol (OAuth)"),
                ("gpt-5.6-terra", "GPT 5.6 Terra (OAuth)"),
                ("gpt-5.6-luna", "GPT 5.6 Luna (OAuth)"),
                ("gpt-5.5", "GPT 5.5 (OAuth)"),
                ("gpt-5.5-mini", "GPT 5.5 Mini (OAuth)"),
                ("gpt-5.5-nano", "GPT 5.5 Nano (OAuth)"),
                ("gpt-5.3-codex", "GPT 5.3 Codex (OAuth)"),
                ("gpt-5.3-codex-spark", "GPT 5.3 Codex Spark (OAuth)"),
                ("gpt-5.2-codex", "GPT 5.2 Codex (OAuth)"),
            ],
        );
        group(
            "xai",
            ProviderApi::OpenAiCompletions,
            AuthMode::OAuth,
            Some("xai"),
            &[
                ("grok-4.6", "Grok 4.6 (SuperGrok OAuth)"),
                ("grok-4.5", "Grok 4.5 (SuperGrok OAuth)"),
            ],
        );
        group(
            "fireworks",
            ProviderApi::OpenAiCompletions,
            AuthMode::ApiKey,
            None,
            &[
                ("accounts/fireworks/models/glm-5p2", "GLM 5.2"),
                ("accounts/fireworks/models/kimi-k2p7-code", "Kimi K2.7 Code"),
            ],
        );
        v
    }

    pub fn load_from_path(path: &Path) -> Result<Self, ModelRegistryError> {
        Ok(Self::from_file_records(Self::load_file_records(path)?))
    }

    /// The effective registry (built-in table + the given user-file records),
    /// built from records already parsed elsewhere so one on-disk read can
    /// feed both the catalogue resolve and the runtime composition (#1193).
    pub fn from_file_records(records: Vec<ModelRecord>) -> Self {
        let mut registry = Self::builtin();
        for record in records {
            registry.upsert(record);
        }
        registry
    }

    /// Parse only the records `path` itself defines (no built-in table), so
    /// the catalogue source adapters can feed the user-defined layer
    /// separately from the built-in layer. A missing file is an empty layer.
    pub fn load_file_records(path: &Path) -> Result<Vec<ModelRecord>, ModelRegistryError> {
        Ok(Self::load_registry_config(path)?.records)
    }

    /// Parse the full user registry file: the model records it defines plus
    /// the per-provider connection defaults (also for providers that list no
    /// models). One typed parse is the single truth for `models.json`, so the
    /// discovery/refresh path and the runtime never re-interpret the file with
    /// diverging ad-hoc rules (epic #1193 slice-4 review). A missing file is
    /// an empty configuration.
    pub fn load_registry_config(path: &Path) -> Result<RegistryConfig, ModelRegistryError> {
        let mut records = Vec::new();
        let mut provider_defaults = Vec::new();
        if !path.exists() {
            return Ok(RegistryConfig::default());
        }
        let content = std::fs::read_to_string(path).map_err(ModelRegistryError::Io)?;
        let file: RegistryFile =
            serde_json::from_str(&content).map_err(ModelRegistryError::Parse)?;
        // Deterministic provider order regardless of the map's iteration.
        let mut providers: Vec<_> = file.providers.into_iter().collect();
        providers.sort_by(|a, b| a.0.cmp(&b.0));
        let mut unsupported = Vec::new();
        let mut skipped = Vec::new();
        for (provider_key, provider) in providers {
            let declared_api = provider.api.as_deref().unwrap_or("openai-completions");
            let api = match ProviderApi::parse(declared_api) {
                Ok(api) => api,
                // A transport this build has no adapter for must not fail the
                // whole file: the provider's models stay known-but-unrunnable
                // catalogue entries with a structured reason (#1575, AC3).
                Err(ModelRegistryError::UnknownApi(declared)) => {
                    unsupported.push(UnsupportedProviderConfig {
                        provider: provider_key,
                        declared_transport: declared,
                        models: provider
                            .models
                            .into_iter()
                            .map(|m| (m.id, m.name))
                            .collect(),
                    });
                    continue;
                }
                Err(error) => return Err(error),
            };

            // Resolve the auth mode. An explicit `auth` block wins; otherwise we
            // default to ApiKey (the historical behaviour). The block's apiKey
            // (when present) takes precedence over the legacy top-level apiKey.
            let env = |name: &str| std::env::var(name).ok();
            let (auth, oauth_provider, block_api_key) = match provider.auth {
                Some(block) => {
                    let mode = block.mode.as_deref().unwrap_or("apiKey");
                    match mode {
                        "apiKey" | "api_key" => (
                            AuthMode::ApiKey,
                            None,
                            block.api_key.map(|v| resolve_registry_value(&v, env)),
                        ),
                        "oauth" => (AuthMode::OAuth, block.oauth_provider, None),
                        // An unknown auth mode must not fail the whole file
                        // (the same per-block degradation AC3 gives an
                        // unknown `api`): this provider's models are skipped
                        // with a diagnostic and the other blocks still load
                        // (#1581 review).
                        other => {
                            skipped.push(SkippedProviderBlock {
                                provider: provider_key.clone(),
                                error: format!(
                                    "provider '{provider_key}' declares unknown auth mode '{other}' (supported: apiKey, oauth); its models were skipped"
                                ),
                            });
                            continue;
                        }
                    }
                }
                None => (AuthMode::ApiKey, None, None),
            };

            let base_url = provider.base_url.or(provider.api_base);
            let api_key =
                block_api_key.or_else(|| provider.api_key.map(|v| resolve_registry_value(&v, env)));
            let auth_header = provider.auth_header.unwrap_or(true);
            let allow_remote_http = provider.allow_remote_http.unwrap_or(false);
            let defaults = ProviderDefaults {
                api,
                auth,
                oauth_provider: oauth_provider.clone(),
                base_url: base_url.clone(),
                api_key: api_key.clone(),
                auth_header,
                allow_remote_http,
            };
            for model in provider.models {
                let mut record = ModelRecord::with_defaults(
                    &provider_key,
                    &model.id,
                    model.name.as_deref(),
                    api,
                );
                record.base_url = base_url.clone();
                record.api_key = api_key.clone();
                record.auth_header = auth_header;
                record.allow_remote_http = allow_remote_http;
                record.auth = auth;
                record.oauth_provider = oauth_provider.clone();
                if let Some(input) = model.input {
                    record.input = input;
                }
                if let Some(v) = model.context_window {
                    record.context_window = v;
                    record.context_window_explicit = true;
                }
                if let Some(v) = model.max_tokens {
                    record.max_tokens = v;
                    record.max_tokens_explicit = true;
                }
                if let Some(v) = model.reasoning {
                    record.reasoning = v;
                }
                if let Some(cost) = model.cost {
                    record.cost = ModelCost {
                        input: cost.input.unwrap_or(0.0),
                        output: cost.output.unwrap_or(0.0),
                        cache_read: cost.cache_read.or(cost.cache_read_camel).unwrap_or(0.0),
                        cache_write: cost.cache_write.or(cost.cache_write_camel).unwrap_or(0.0),
                    };
                }
                records.push(record);
            }
            provider_defaults.push((provider_key, defaults));
        }
        let mut overrides: Vec<(String, ModelOverride)> = file
            .overrides
            .into_iter()
            .map(|(qualified, patch)| {
                (
                    qualified,
                    ModelOverride {
                        name: patch.name,
                        context_window: patch.context_window,
                        max_tokens: patch.max_tokens,
                        api_key: patch.api_key,
                    },
                )
            })
            .collect();
        overrides.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(RegistryConfig {
            records,
            providers: provider_defaults,
            overrides,
            unsupported,
            skipped,
        })
    }

    pub fn find(&self, provider: &str, id: &str) -> Option<&ModelRecord> {
        self.models
            .iter()
            .find(|m| m.provider == provider && m.id == id)
    }

    pub fn models(&self) -> &[ModelRecord] {
        &self.models
    }

    /// The registry output cap for a `provider/id` qualified model string, if
    /// the model is known (#935). Used to clamp the effective per-request
    /// output tokens to `min(configured_max_tokens, model.max_tokens)`. Returns
    /// `None` when the model is unknown, the id is not `provider/id`-shaped, or
    /// the model does not declare an explicit `maxTokens` (a synthesized default
    /// is not a real output limit and must not clamp), so callers fall back to
    /// the configured value (no clamp).
    pub fn max_tokens_for(&self, qualified: &str) -> Option<u32> {
        let (provider, id) = qualified.split_once('/')?;
        self.find(provider, id)
            .filter(|m| m.max_tokens_explicit)
            .map(|m| m.max_tokens)
    }

    /// The known context window for a `provider/id` qualified model string
    /// (#1044). Returns `None` when the model is unknown, the id is not
    /// `provider/id`-shaped, or the model does not declare an explicit
    /// `contextWindow` (a synthesized default is not a real window and must
    /// not clamp), so callers fall back to the configured budget.
    pub fn context_window_for(&self, qualified: &str) -> Option<usize> {
        let (provider, id) = qualified.split_once('/')?;
        self.find(provider, id)
            .filter(|m| m.context_window_explicit)
            .map(|m| m.context_window as usize)
    }

    fn upsert(&mut self, record: ModelRecord) {
        if let Some(existing) = self
            .models
            .iter_mut()
            .find(|m| m.provider == record.provider && m.id == record.id)
        {
            *existing = record;
        } else {
            self.models.push(record);
        }
    }
}

impl ModelRecord {
    fn with_defaults(provider: &str, id: &str, name: Option<&str>, api: ProviderApi) -> Self {
        Self {
            provider: provider.to_string(),
            id: id.to_string(),
            display_name: name.map(str::to_string),
            api,
            base_url: None,
            api_key: None,
            auth_header: true,
            allow_remote_http: false,
            input: vec!["text".to_string()],
            context_window: DEFAULT_CONTEXT_WINDOW,
            max_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
            max_tokens_explicit: false,
            context_window_explicit: false,
            cost: ModelCost::default(),
            reasoning: false,
            auth: AuthMode::ApiKey,
            oauth_provider: None,
        }
    }

    pub fn qualified_id(&self) -> String {
        format!("{}/{}", self.provider, self.id)
    }
}

/// Per-provider connection defaults parsed from the user registry file,
/// available also for providers that list no models. The single typed parse
/// feeding these is shared with the record loading, so every consumer of
/// `models.json` (runtime, catalogue, discovery/refresh) sees one semantics.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderDefaults {
    pub api: ProviderApi,
    pub auth: AuthMode,
    pub oauth_provider: Option<String>,
    pub base_url: Option<String>,
    /// The resolved key value (env interpolation applied). Secret material —
    /// never render it.
    pub api_key: Option<String>,
    pub auth_header: bool,
    pub allow_remote_http: bool,
}

impl ProviderDefaults {
    /// Synthesize a full model record for a model discovered under this
    /// provider (catalogue refresh, epic #1193 slice 4): connection and auth
    /// come from the provider's configuration; listing metadata uses the
    /// registry's synthesized defaults, exactly like a models.json entry that
    /// declares only an id.
    pub fn record_for(&self, provider_key: &str, id: &str, name: Option<&str>) -> ModelRecord {
        let mut record = ModelRecord::with_defaults(provider_key, id, name, self.api);
        record.base_url = self.base_url.clone();
        record.api_key = self.api_key.clone();
        record.auth_header = self.auth_header;
        record.allow_remote_http = self.allow_remote_http;
        record.auth = self.auth;
        record.oauth_provider = self.oauth_provider.clone();
        record
    }
}

/// Synthesized capability defaults for an entry that declares only an id:
/// the single truth shared by `ModelRecord::with_defaults`, the discovery
/// cache mapping, and unsupported-transport entries, so the layers can never
/// drift apart (#1581 review). Marked non-explicit downstream, so nothing
/// clamps on these values.
pub(crate) const DEFAULT_CONTEXT_WINDOW: u32 = 128_000;
pub(crate) const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 16_384;

/// The full parsed user registry file: model records plus per-provider
/// defaults, in deterministic (sorted) provider order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RegistryConfig {
    pub records: Vec<ModelRecord>,
    pub providers: Vec<(String, ProviderDefaults)>,
    /// Stable-ID metadata overrides from the file's `overrides` section, in
    /// deterministic (sorted) qualified-id order (#1575, AC1).
    pub overrides: Vec<(String, ModelOverride)>,
    /// Providers whose declared transport has no adapter in this build: their
    /// models become known-but-unrunnable catalogue entries instead of
    /// erasing the file (#1575, AC3).
    pub unsupported: Vec<UnsupportedProviderConfig>,
    /// Provider blocks skipped with a diagnostic (e.g. an unknown auth mode)
    /// so one bad block never erases the file's valid neighbours.
    pub skipped: Vec<SkippedProviderBlock>,
}

/// One provider block `load_registry_config` skipped, with the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedProviderBlock {
    pub provider: String,
    pub error: String,
}

/// One stable-ID metadata override (`overrides` section of `models.json`):
/// only the declared fields replace the base entry's metadata. `api_key` is a
/// credential *reference* (`$ENV`); the catalogue layer rejects literals.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModelOverride {
    pub name: Option<String>,
    pub context_window: Option<u32>,
    pub max_tokens: Option<u32>,
    pub api_key: Option<String>,
}

/// A provider block whose declared transport this build cannot run, kept so
/// the catalogue can list its models as known with a structured reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedProviderConfig {
    pub provider: String,
    pub declared_transport: String,
    /// `(model id, display name)` pairs the block listed.
    pub models: Vec<(String, Option<String>)>,
}

pub fn resolve_registry_value<F>(value: &str, env: F) -> String
where
    F: Fn(&str) -> Option<String>,
{
    let mut out = String::new();
    let chars: Vec<char> = value.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '$' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        if i + 1 < chars.len() && chars[i + 1] == '$' {
            out.push('$');
            i += 2;
            continue;
        }
        if i + 1 < chars.len() && chars[i + 1] == '{' {
            if let Some(end) = chars[i + 2..].iter().position(|c| *c == '}') {
                let name: String = chars[i + 2..i + 2 + end].iter().collect();
                out.push_str(&env(&name).unwrap_or_default());
                i += end + 3;
                continue;
            }
        }
        let start = i + 1;
        let mut end = start;
        while end < chars.len() && (chars[end].is_ascii_alphanumeric() || chars[end] == '_') {
            end += 1;
        }
        if end == start {
            out.push('$');
            i += 1;
        } else {
            let name: String = chars[start..end].iter().collect();
            out.push_str(&env(&name).unwrap_or_default());
            i = end;
        }
    }
    out
}

#[derive(Deserialize)]
struct RegistryFile {
    #[serde(default)]
    providers: HashMap<String, RegistryProvider>,
    /// Stable-ID metadata overrides keyed by qualified `provider/model` id.
    #[serde(default)]
    overrides: HashMap<String, RegistryOverride>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegistryOverride {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    context_window: Option<u32>,
    #[serde(default)]
    max_tokens: Option<u32>,
    /// Credential reference (`$ENV`); literals are rejected downstream.
    #[serde(default)]
    api_key: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegistryProvider {
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    api: Option<String>,
    #[serde(default)]
    auth_header: Option<bool>,
    #[serde(default)]
    allow_remote_http: Option<bool>,
    #[serde(default)]
    auth: Option<RegistryAuth>,
    #[serde(default)]
    models: Vec<RegistryModel>,
}

/// Explicit auth declaration for a registry provider.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegistryAuth {
    /// "apiKey" (default) or "oauth".
    #[serde(default)]
    mode: Option<String>,
    /// For `apiKey` mode: the key (supports `$ENV` interpolation).
    #[serde(default)]
    api_key: Option<String>,
    /// For `oauth` mode: the kernel OAuth provider identity to resolve against.
    #[serde(default)]
    oauth_provider: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegistryModel {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    reasoning: Option<bool>,
    #[serde(default)]
    input: Option<Vec<String>>,
    #[serde(default)]
    context_window: Option<u32>,
    #[serde(default)]
    max_tokens: Option<u32>,
    #[serde(default)]
    cost: Option<RegistryCost>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegistryCost {
    #[serde(default)]
    input: Option<f64>,
    #[serde(default)]
    output: Option<f64>,
    #[serde(default, rename = "cacheRead")]
    cache_read_camel: Option<f64>,
    #[serde(default, rename = "cacheWrite")]
    cache_write_camel: Option<f64>,
    #[serde(default, rename = "cache_read")]
    cache_read: Option<f64>,
    #[serde(default, rename = "cache_write")]
    cache_write: Option<f64>,
}

#[path = "model_registry_gpt56_pricing.rs"]
mod gpt56_pricing;
use gpt56_pricing::gpt_5_6_cost;

#[cfg(test)]
#[path = "model_registry_tests.rs"]
mod tests;
