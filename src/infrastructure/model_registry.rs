use serde::Deserialize;
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
    pub input: Vec<String>,
    pub context_window: u32,
    pub max_tokens: u32,
    pub cost: ModelCost,
    pub reasoning: bool,
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
}

impl std::fmt::Display for ModelRegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "failed to read models registry: {e}"),
            Self::Parse(e) => write!(f, "failed to parse models registry: {e}"),
            Self::UnknownApi(api) => write!(f, "unknown api '{api}' in models registry"),
        }
    }
}

impl std::error::Error for ModelRegistryError {}

impl ModelRegistry {
    pub fn builtin() -> Self {
        let mut r = Self { models: Vec::new() };
        for (provider, id, name, api) in [
            (
                "anthropic",
                "claude-fable-5",
                "Claude Fable 5",
                ProviderApi::AnthropicMessages,
            ),
            (
                "anthropic",
                "claude-opus-4-8",
                "Claude Opus 4.8",
                ProviderApi::AnthropicMessages,
            ),
            (
                "anthropic",
                "claude-opus-4-7",
                "Claude Opus 4.7",
                ProviderApi::AnthropicMessages,
            ),
            (
                "anthropic",
                "claude-opus-4-6",
                "Claude Opus 4.6",
                ProviderApi::AnthropicMessages,
            ),
            (
                "anthropic",
                "claude-opus-4-5",
                "Claude Opus 4.5",
                ProviderApi::AnthropicMessages,
            ),
            (
                "anthropic",
                "claude-sonnet-4-6",
                "Claude Sonnet 4.6",
                ProviderApi::AnthropicMessages,
            ),
            (
                "anthropic",
                "claude-sonnet-4-5",
                "Claude Sonnet 4.5",
                ProviderApi::AnthropicMessages,
            ),
            (
                "openai",
                "gpt-5.5",
                "GPT 5.5",
                ProviderApi::OpenAiCompletions,
            ),
            (
                "openai",
                "gpt-5.5-mini",
                "GPT 5.5 Mini",
                ProviderApi::OpenAiCompletions,
            ),
            (
                "openai",
                "gpt-5.5-nano",
                "GPT 5.5 Nano",
                ProviderApi::OpenAiCompletions,
            ),
            (
                "openai",
                "gpt-5.3-codex",
                "GPT 5.3 Codex",
                ProviderApi::OpenAiCompletions,
            ),
            (
                "openai",
                "gpt-5.3-codex-spark",
                "GPT 5.3 Codex Spark",
                ProviderApi::OpenAiCompletions,
            ),
            (
                "openai",
                "gpt-5.2-codex",
                "GPT 5.2 Codex",
                ProviderApi::OpenAiCompletions,
            ),
        ] {
            r.upsert(ModelRecord::with_defaults(provider, id, Some(name), api));
        }
        r
    }

    pub fn load_from_path(path: &Path) -> Result<Self, ModelRegistryError> {
        let mut registry = Self::builtin();
        if !path.exists() {
            return Ok(registry);
        }
        let content = std::fs::read_to_string(path).map_err(ModelRegistryError::Io)?;
        let file: RegistryFile =
            serde_json::from_str(&content).map_err(ModelRegistryError::Parse)?;
        for (provider_key, provider) in file.providers {
            let api = ProviderApi::parse(provider.api.as_deref().unwrap_or("openai-completions"))?;
            let base_url = provider.base_url.or(provider.api_base);
            let api_key = provider
                .api_key
                .map(|v| resolve_registry_value(&v, |name| std::env::var(name).ok()));
            let auth_header = provider.auth_header.unwrap_or(true);
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
                if let Some(input) = model.input {
                    record.input = input;
                }
                if let Some(v) = model.context_window {
                    record.context_window = v;
                }
                if let Some(v) = model.max_tokens {
                    record.max_tokens = v;
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
                registry.upsert(record);
            }
        }
        Ok(registry)
    }

    pub fn find(&self, provider: &str, id: &str) -> Option<&ModelRecord> {
        self.models
            .iter()
            .find(|m| m.provider == provider && m.id == id)
    }

    pub fn models(&self) -> &[ModelRecord] {
        &self.models
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
            input: vec!["text".to_string()],
            context_window: 128_000,
            max_tokens: 16_384,
            cost: ModelCost::default(),
            reasoning: false,
        }
    }

    pub fn qualified_id(&self) -> String {
        format!("{}/{}", self.provider, self.id)
    }
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
    models: Vec<RegistryModel>,
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

#[cfg(test)]
#[path = "model_registry_tests.rs"]
mod tests;
