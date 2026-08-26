//! Serde wire structs for the user `models.json` file format, split out of
//! `model_registry.rs` (which owns parsing and semantics) to keep that file
//! within the repository line-count baseline.

use std::collections::HashMap;

use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct RegistryFile {
    #[serde(default)]
    pub(super) providers: HashMap<String, RegistryProvider>,
    /// Stable-ID metadata overrides keyed by qualified `provider/model` id.
    #[serde(default)]
    pub(super) overrides: HashMap<String, RegistryOverride>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RegistryOverride {
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) context_window: Option<u32>,
    #[serde(default)]
    pub(super) max_tokens: Option<u32>,
    /// Credential reference (`$ENV`); literals are rejected downstream.
    #[serde(default)]
    pub(super) api_key: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RegistryProvider {
    #[serde(default)]
    pub(super) base_url: Option<String>,
    #[serde(default)]
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) api_key: Option<String>,
    #[serde(default)]
    pub(super) api: Option<String>,
    #[serde(default)]
    pub(super) auth_header: Option<bool>,
    #[serde(default)]
    pub(super) allow_remote_http: Option<bool>,
    #[serde(default)]
    pub(super) auth: Option<RegistryAuth>,
    #[serde(default)]
    pub(super) models: Vec<RegistryModel>,
}

/// Explicit auth declaration for a registry provider.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RegistryAuth {
    /// "apiKey" (default) or "oauth".
    #[serde(default)]
    pub(super) mode: Option<String>,
    /// For `apiKey` mode: the key (supports `$ENV` interpolation).
    #[serde(default)]
    pub(super) api_key: Option<String>,
    /// For `oauth` mode: the kernel OAuth provider identity to resolve against.
    #[serde(default)]
    pub(super) oauth_provider: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RegistryModel {
    pub(super) id: String,
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) reasoning: Option<bool>,
    #[serde(default)]
    pub(super) input: Option<Vec<String>>,
    #[serde(default)]
    pub(super) context_window: Option<u32>,
    #[serde(default)]
    pub(super) max_tokens: Option<u32>,
    #[serde(default)]
    pub(super) cost: Option<RegistryCost>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RegistryCost {
    #[serde(default)]
    pub(super) input: Option<f64>,
    #[serde(default)]
    pub(super) output: Option<f64>,
    #[serde(default, rename = "cacheRead")]
    pub(super) cache_read_camel: Option<f64>,
    #[serde(default, rename = "cacheWrite")]
    pub(super) cache_write_camel: Option<f64>,
    #[serde(default, rename = "cache_read")]
    pub(super) cache_read: Option<f64>,
    #[serde(default, rename = "cache_write")]
    pub(super) cache_write: Option<f64>,
}
