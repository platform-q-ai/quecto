//! Infrastructure discovery adapters for the catalogue refresh port
//! (epic #1193, slice 4).
//!
//! Discovery results persist as a *source cache* in the generated-data layer
//! — never by rewriting user-owned catalogue data. Ordinary loads read only
//! the cache (network-free); network happens inside the refresh path.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::application::ports::{
    CatalogueSource, RefreshChange, RefreshContext, RefreshError, RefreshRedactionPort,
    RefreshableCatalogueSource, SkippedRecord, SourceEntries,
};
use crate::domain::catalogue::{
    AuthIdentity, Availability, CatalogueEntry, ModelCapabilities, ModelCost, ModelDescriptor,
    ModelRef, ProviderDescriptor, ProviderId, SourceLayer, TransportKind,
};
use crate::infrastructure::atomic_write::atomic_write;
use crate::infrastructure::model_registry::{AuthMode, ProviderApi, ProviderDefaults};
use crate::infrastructure::providers::{
    ProviderFactoryError, validate_provider_api_base_with_options,
};

/// Cap on distinct models one discovery response may declare; a compromised
/// endpoint must not be able to grow the cache without bound.
const MAX_DISCOVERED_MODELS: usize = 10_000;

/// Subdirectory of the base dir holding per-provider discovery caches
/// (generated data, distinct from the user-owned `models.json`).
pub const DISCOVERY_CACHE_DIR: &str = "discovered";

/// Where the per-provider discovery caches for `base_dir` live.
pub fn discovery_cache_dir(base_dir: &Path) -> PathBuf {
    base_dir.join(DISCOVERY_CACHE_DIR)
}

/// One cached, mapped model from a provider's `/models` listing. Only these
/// mapped fields ever reach disk — never the raw response body.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct CachedModel {
    id: String,
    name: String,
}

/// Persisted per-provider discovery cache that doubles as a discovered-layer
/// catalogue source.
pub struct DiscoverySourceCache {
    dir: PathBuf,
    provider: String,
    id: String,
}

impl DiscoverySourceCache {
    pub fn new(dir: &Path, provider: &str) -> Self {
        Self {
            dir: dir.to_path_buf(),
            provider: provider.to_string(),
            id: format!("discovered:{provider}"),
        }
    }

    /// The provider this cache belongs to.
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// Where this provider's cache lives on disk.
    pub fn cache_path(&self) -> PathBuf {
        self.dir.join(format!("{}.json", self.provider))
    }

    /// Map an OpenAI-compatible `/models` response body into catalogue
    /// entries and atomically persist them as this provider's source cache.
    /// Only mapped model entries are persisted — never the raw response, so
    /// credential material a server might echo can never reach disk. A body
    /// that fails to map leaves any previous cache untouched. When the mapped
    /// bytes equal the existing cache the write is skipped entirely (no
    /// steady-state write amplification for stable providers).
    /// Returns the refresh change, carrying the number of models cached.
    pub fn store_models_response(&self, body: &str) -> Result<RefreshChange, String> {
        if !safe_cache_key(&self.provider) {
            // Defense in depth: the provider key becomes the cache file name,
            // so a key that could traverse outside the cache dir must never
            // reach the filesystem (slice-4 review).
            return Err(unsafe_key_reason(&self.provider));
        }
        let models = map_models_response(body)?;
        let bytes = serde_json::to_vec_pretty(&models)
            .map_err(|e| format!("failed to serialize discovery cache: {e}"))?;
        let path = self.cache_path();
        if let Ok(existing) = std::fs::read(&path)
            && existing == bytes
        {
            return Ok(RefreshChange::Unchanged {
                models: models.len(),
            });
        }
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| format!("failed to create {}: {e}", self.dir.display()))?;
        atomic_write(&path, &bytes, Some(0o600))
            .map_err(|e| format!("failed to write {} atomically: {e}", path.display()))?;
        Ok(RefreshChange::Updated {
            models: models.len(),
        })
    }
}

/// Whether a provider key is safe to use as a cache file stem: it must stay a
/// single plain path component, so a hostile or mistyped key in `models.json`
/// can never write outside the discovery cache dir, and every cache written
/// is re-enumerable by [`discovery_cache_sources`] (which lists only direct
/// `*.json` children).
fn safe_cache_key(key: &str) -> bool {
    !key.is_empty()
        && !key.starts_with('.')
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn unsafe_key_reason(key: &str) -> String {
    format!(
        "provider key '{key}' cannot name a discovery cache file (allowed: ASCII letters, digits, '-', '_', '.'; must not start with '.'); rename the provider in models.json"
    )
}

/// Map a `/models` response body into cached models, sorted by id with
/// duplicates collapsed. Fails without side effects on any malformed record.
fn map_models_response(body: &str) -> Result<Vec<CachedModel>, String> {
    let parsed: Value =
        serde_json::from_str(body).map_err(|e| format!("model list response is not JSON: {e}"))?;
    let data = parsed
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| "model list response missing data array".to_string())?;
    if data.len() > MAX_DISCOVERED_MODELS {
        return Err(format!(
            "model catalog contains more than {MAX_DISCOVERED_MODELS} entries"
        ));
    }
    let mut by_id = std::collections::BTreeMap::new();
    for model in data {
        let id = model
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| "model entry missing string id".to_string())?;
        let name = model
            .get("name")
            .or_else(|| model.get("owned_by"))
            .and_then(Value::as_str)
            .unwrap_or(id);
        by_id.insert(
            id.to_string(),
            CachedModel {
                id: id.to_string(),
                name: name.to_string(),
            },
        );
    }
    Ok(by_id.into_values().collect())
}

impl CatalogueSource for DiscoverySourceCache {
    fn id(&self) -> &str {
        &self.id
    }

    fn layer(&self) -> SourceLayer {
        SourceLayer::Discovered
    }

    fn load(&self) -> Result<SourceEntries, String> {
        let path = self.cache_path();
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            // No cache yet is the ordinary pre-first-refresh state.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(SourceEntries::default());
            }
            Err(e) => return Err(format!("failed to read {}: {e}", path.display())),
        };
        let models: Vec<CachedModel> = serde_json::from_str(&text)
            .map_err(|e| format!("failed to parse {}: {e}", path.display()))?;
        let mut entries = SourceEntries::default();
        for model in models {
            match cached_entry(&self.provider, &model) {
                Ok(entry) => entries.entries.push(entry),
                Err(error) => entries.skipped.push(SkippedRecord {
                    record: format!("{}/{}", self.provider, model.id),
                    error,
                }),
            }
        }
        Ok(entries)
    }
}

/// A discovered model as a domain catalogue entry. Discovery listings carry
/// no capability metadata, so limits are the registry's synthesized defaults
/// (marked non-explicit); user layers override these in normal precedence.
fn cached_entry(provider: &str, model: &CachedModel) -> Result<CatalogueEntry, String> {
    let provider_id = ProviderId::new(provider.to_string()).map_err(|e| e.to_string())?;
    let reference =
        ModelRef::parse(provider.to_string(), model.id.clone()).map_err(|e| e.to_string())?;
    Ok(CatalogueEntry {
        provider: ProviderDescriptor {
            id: provider_id,
            display_name: Some(provider.to_string()),
            transport: TransportKind::OpenAiCompletions,
            auth: AuthIdentity::ApiKey,
        },
        model: ModelDescriptor {
            reference,
            display_name: Some(model.name.clone()),
            capabilities: ModelCapabilities {
                input_modalities: vec!["text".to_string()],
                context_window: 128_000,
                max_output_tokens: 16_384,
                context_window_explicit: false,
                max_output_tokens_explicit: false,
                reasoning: false,
                cost: ModelCost::default(),
            },
            availability: Availability::runnable(),
        },
    })
}

/// Read at most `max_bytes` from `reader`; a longer body is an error, never a
/// silent truncation (a truncated JSON body would fail parsing confusingly,
/// and a compromised endpoint must not stream unbounded data).
pub(crate) fn read_capped(reader: impl Read, max_bytes: u64) -> Result<Vec<u8>, String> {
    let mut capped = reader.take(max_bytes + 1);
    let mut bytes = Vec::new();
    capped
        .read_to_end(&mut bytes)
        .map_err(|e| format!("failed while reading response: {e}"))?;
    if bytes.len() as u64 > max_bytes {
        return Err(format!("response body exceeds {max_bytes} bytes"));
    }
    Ok(bytes)
}

/// How an OpenAI-compatible discovery source reaches its provider: the
/// `/models` URL derived from the configured base URL plus optional bearer
/// credential material (values stay here in infrastructure).
#[derive(Debug, Clone)]
pub struct DiscoveryEndpoint {
    pub url: String,
    pub api_key: Option<String>,
}

impl DiscoveryEndpoint {
    /// Derive the `/models` URL for an OpenAI-compatible provider from its
    /// configured base URL, applying the same base-URL validation the
    /// provider factory uses.
    pub fn for_openai_compatible(
        provider_key: &str,
        base_url: &str,
        allow_remote_http: bool,
        api_key: Option<String>,
    ) -> Result<Self, String> {
        validate_provider_api_base_with_options(provider_key, base_url, allow_remote_http, true)
            .map_err(|e| match e {
                ProviderFactoryError::InvalidApiBase { reason, .. } => format!(
                    "provider '{provider_key}' has invalid baseUrl '{}': {reason}",
                    redact_url_for_error(base_url)
                ),
                other => other.to_string(),
            })?;
        let parsed = reqwest::Url::parse(base_url).map_err(|e| {
            format!("provider '{provider_key}' has invalid baseUrl '<invalid url>': {e}")
        })?;
        let base_path = parsed.path().trim_end_matches('/').to_string();
        if !base_path.ends_with("/v1") {
            return Err(format!(
                "provider '{provider_key}' baseUrl must end at an OpenAI-compatible /v1 endpoint"
            ));
        }
        let mut models_url = parsed;
        models_url.set_path(&format!("{base_path}/models"));
        Ok(Self {
            url: models_url.to_string(),
            api_key,
        })
    }
}

fn redact_url_for_error(url: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(mut parsed) => {
            let _ = parsed.set_username("");
            let _ = parsed.set_password(None);
            parsed.set_query(None);
            parsed.set_fragment(None);
            parsed.to_string()
        }
        Err(_) => "<invalid url>".to_string(),
    }
}

/// Fetch an OpenAI-compatible `/models` listing under the refresh bounds and
/// return the raw body text for the cache to map.
fn fetch_models_body(endpoint: &DiscoveryEndpoint, ctx: &RefreshContext) -> Result<String, String> {
    let display_url = redact_url_for_error(&endpoint.url);
    let client = reqwest::blocking::Client::builder()
        .timeout(ctx.bounds.timeout)
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;
    let mut req = client.get(&endpoint.url);
    if let Some(token) = endpoint.api_key.as_deref().filter(|s| !s.is_empty()) {
        req = req.bearer_auth(token);
    }
    let resp = req
        .send()
        .map_err(|e| format!("GET {display_url} failed: {}", e.without_url()))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("GET {display_url} returned {status}"));
    }
    let bytes = read_capped(resp, ctx.bounds.max_response_bytes)
        .map_err(|e| format!("GET {display_url}: {e}"))?;
    String::from_utf8(bytes).map_err(|e| format!("GET {display_url} returned non-UTF-8 data: {e}"))
}

/// A refreshable discovery source for an OpenAI-compatible provider: refresh
/// fetches `/models` under the run's bounds and persists the mapped listing
/// into the provider's [`DiscoverySourceCache`]; ordinary loads read only the
/// cache.
pub struct HttpDiscoverySource {
    cache: DiscoverySourceCache,
    /// The derived endpoint, or why it could not be derived from the
    /// provider's configuration (reported when a refresh is attempted).
    endpoint: Result<DiscoveryEndpoint, String>,
}

impl HttpDiscoverySource {
    pub fn new(cache: DiscoverySourceCache, endpoint: Result<DiscoveryEndpoint, String>) -> Self {
        Self { cache, endpoint }
    }
}

impl CatalogueSource for HttpDiscoverySource {
    fn id(&self) -> &str {
        self.cache.provider()
    }

    fn layer(&self) -> SourceLayer {
        SourceLayer::Discovered
    }

    fn load(&self) -> Result<SourceEntries, String> {
        self.cache.load()
    }
}

impl RefreshableCatalogueSource for HttpDiscoverySource {
    fn refresh(&self, ctx: &RefreshContext) -> Result<RefreshChange, RefreshError> {
        let endpoint = self
            .endpoint
            .as_ref()
            .map_err(|reason| RefreshError::Failed {
                reason: reason.clone(),
            })?;
        let body =
            fetch_models_body(endpoint, ctx).map_err(|reason| RefreshError::Failed { reason })?;
        self.cache
            .store_models_response(&body)
            .map_err(|reason| RefreshError::Failed { reason })
    }
}

/// A configured catalogue source whose provider exposes no model listing to
/// refresh from (e.g. Anthropic). Refresh always reports an actionable
/// unsupported outcome; loads contribute nothing.
pub struct UnsupportedRefreshSource {
    provider: String,
    reason: String,
}

impl UnsupportedRefreshSource {
    pub fn new(provider: &str, reason: impl Into<String>) -> Self {
        Self {
            provider: provider.to_string(),
            reason: reason.into(),
        }
    }
}

impl CatalogueSource for UnsupportedRefreshSource {
    fn id(&self) -> &str {
        &self.provider
    }

    fn layer(&self) -> SourceLayer {
        SourceLayer::Discovered
    }

    fn load(&self) -> Result<SourceEntries, String> {
        Ok(SourceEntries::default())
    }
}

impl RefreshableCatalogueSource for UnsupportedRefreshSource {
    fn refresh(&self, _ctx: &RefreshContext) -> Result<RefreshChange, RefreshError> {
        Err(RefreshError::Unsupported {
            reason: self.reason.clone(),
        })
    }
}

/// Redacts every configured credential value from human-facing refresh text.
pub struct SecretsRedaction {
    secrets: Vec<String>,
}

impl SecretsRedaction {
    pub fn new(secrets: Vec<String>) -> Self {
        Self {
            // Redact only real secret material: empty strings would match
            // everywhere and one-character "secrets" are not credentials.
            secrets: secrets.into_iter().filter(|s| s.len() > 1).collect(),
        }
    }
}

impl RefreshRedactionPort for SecretsRedaction {
    fn redact(&self, text: &str) -> String {
        let mut redacted = text.to_string();
        for secret in &self.secrets {
            redacted = redacted.replace(secret, "[redacted]");
        }
        redacted
    }
}

/// The refreshable discovery sources configured in `base_dir`'s
/// `models.json`, plus the credential values a redactor must strip from any
/// refresh outcome text.
pub struct ConfiguredDiscovery {
    pub sources: Vec<Box<dyn RefreshableCatalogueSource>>,
    pub secrets: Vec<String>,
}

/// Map the typed provider configuration from one `models.json` parse into
/// refreshable catalogue sources: OpenAI-compatible api-key providers refresh
/// over HTTP into their discovery cache; every other configuration reports an
/// actionable unsupported outcome. Taking the already-parsed
/// [`ProviderDefaults`] (rather than re-reading the file) keeps one typed
/// parse as the single truth for the file's semantics and lets the caller
/// feed refresh and resolve from the same on-disk read (slice-4 review).
pub fn configured_discovery(
    base_dir: &Path,
    providers: &[(String, ProviderDefaults)],
) -> ConfiguredDiscovery {
    let cache_dir = discovery_cache_dir(base_dir);
    let mut sources: Vec<Box<dyn RefreshableCatalogueSource>> = Vec::new();
    let mut secrets = Vec::new();
    for (key, defaults) in providers {
        // Collect every configured secret for redaction regardless of which
        // source shape the provider maps to.
        if let Some(secret) = &defaults.api_key {
            secrets.push(secret.clone());
        }
        sources.push(provider_discovery_source(&cache_dir, key, defaults));
    }
    ConfiguredDiscovery { sources, secrets }
}

/// Map one configured provider into its refreshable source.
fn provider_discovery_source(
    cache_dir: &Path,
    key: &str,
    defaults: &ProviderDefaults,
) -> Box<dyn RefreshableCatalogueSource> {
    if defaults.api != ProviderApi::OpenAiCompletions {
        return Box::new(UnsupportedRefreshSource::new(
            key,
            "provider api does not expose an OpenAI-compatible model listing endpoint; maintain its models in models.json directly",
        ));
    }
    if defaults.auth == AuthMode::OAuth {
        return Box::new(UnsupportedRefreshSource::new(
            key,
            "provider uses oauth auth, which catalogue refresh does not support; maintain its models in models.json directly",
        ));
    }
    if !safe_cache_key(key) {
        return Box::new(HttpDiscoverySource::new(
            DiscoverySourceCache::new(cache_dir, key),
            Err(unsafe_key_reason(key)),
        ));
    }
    let Some(base_url) = defaults.base_url.as_deref() else {
        return Box::new(UnsupportedRefreshSource::new(
            key,
            "provider declares no baseUrl to discover models from; add one or maintain its models in models.json directly",
        ));
    };
    // Validate the URL before attaching credential material: an unsafe
    // endpoint must never cause a secret to be resolved for it.
    let endpoint =
        DiscoveryEndpoint::for_openai_compatible(key, base_url, defaults.allow_remote_http, None)
            .map(|mut endpoint| {
                endpoint.api_key = defaults.api_key.clone();
                endpoint
            });
    Box::new(HttpDiscoverySource::new(
        DiscoverySourceCache::new(cache_dir, key),
        endpoint,
    ))
}

/// The persisted discovery caches under `base_dir`, as discovered-layer
/// catalogue sources for ordinary (network-free) resolves.
pub fn discovery_cache_sources(base_dir: &Path) -> Vec<DiscoverySourceCache> {
    let dir = discovery_cache_dir(base_dir);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut providers: Vec<String> = entries
        .filter_map(|entry| {
            let name = entry.ok()?.file_name();
            let name = name.to_str()?;
            Some(name.strip_suffix(".json")?.to_string())
        })
        .collect();
    providers.sort();
    providers
        .into_iter()
        .map(|provider| DiscoverySourceCache::new(&dir, &provider))
        .collect()
}

#[cfg(test)]
#[path = "catalogue_discovery_tests.rs"]
mod tests;
