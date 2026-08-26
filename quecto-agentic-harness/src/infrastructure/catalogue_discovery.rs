//! Catalogue refresh/discovery infrastructure adapters.
//!
//! Network discovery, provider-specific URL validation, response bounds, and
//! `models.json` publication are infrastructure concerns. Interfaces call the
//! application refresh use case with this adapter rather than owning discovery
//! behaviour themselves.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use crate::application::ports::{
    CatalogueRefreshAllPort, CatalogueRefreshOutcome, CatalogueRefreshPort, CatalogueRefreshStatus,
};
use crate::infrastructure::atomic_write::atomic_write;
use crate::infrastructure::model_registry::resolve_registry_value;
use crate::infrastructure::providers::{
    ProviderFactoryError, validate_provider_api_base_with_options,
};

// Keep unattended discovery bounded: provider catalogs are small JSON lists,
// while compromised endpoints can otherwise stream arbitrary bytes/items.
pub(crate) const MAX_MODEL_DISCOVERY_RESPONSE_BYTES: usize = 5 * 1024 * 1024;
pub(crate) const MAX_MODEL_DISCOVERY_MODELS: usize = 10_000;

struct ModelsJsonPublishLock {
    file: std::fs::File,
}

impl ModelsJsonPublishLock {
    /// How long to wait for another process's publication to finish.
    const LOCK_WAIT: Duration = Duration::from_secs(5);

    // `File::lock` stabilized in 1.89, which the crate already depends on for
    // the credential and session locks; clippy.toml's declared 1.85 predates it.
    #[expect(clippy::incompatible_msrv)]
    fn acquire(base_dir: &Path) -> Result<Self, String> {
        let lock_path = base_dir.join("models.json.lock");
        // Mode 0600 like the credential lock: a world-readable lock file would
        // let any co-resident user take the exclusive lock and wedge catalogue
        // publication indefinitely.
        #[cfg(unix)]
        let opened = {
            use std::os::unix::fs::OpenOptionsExt;
            OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .mode(0o600)
                .open(&lock_path)
        };
        #[cfg(not(unix))]
        let opened = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path);
        let file = opened.map_err(|e| format!("failed to open {}: {e}", lock_path.display()))?;
        // Bounded: another process holding the lock must not stall discovery (and
        // with it the command loop that awaits it) for an unbounded time.
        let deadline = std::time::Instant::now() + Self::LOCK_WAIT;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Self { file }),
                Err(std::fs::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(std::fs::TryLockError::WouldBlock) => {
                    return Err(format!(
                        "timed out waiting for {} (another process is publishing)",
                        lock_path.display()
                    ));
                }
                Err(e) => {
                    return Err(format!("failed to lock {}: {e}", lock_path.display()));
                }
            }
        }
    }
}

impl Drop for ModelsJsonPublishLock {
    #[expect(clippy::incompatible_msrv)]
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

#[derive(Debug, Clone)]
pub struct ModelsJsonCatalogueRefreshAdapter {
    base_dir: PathBuf,
}

impl ModelsJsonCatalogueRefreshAdapter {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    pub fn refresh_all(&self) -> Vec<CatalogueRefreshOutcome> {
        refresh_all_from_models_json(self, &self.base_dir.join("models.json"))
    }

    /// Wall-clock budget for refreshing every source. Discovery runs on the
    /// command loop, so a catalogue full of unreachable endpoints must not stall
    /// every other UDS command for `sources × request timeout`.
    pub(crate) const REFRESH_ALL_BUDGET: Duration = Duration::from_secs(60);
}

impl CatalogueRefreshAllPort for ModelsJsonCatalogueRefreshAdapter {
    fn refresh_all_sources(&self) -> Vec<CatalogueRefreshOutcome> {
        self.refresh_all()
    }
}

impl CatalogueRefreshPort for ModelsJsonCatalogueRefreshAdapter {
    fn refresh_source(&self, source: &str) -> CatalogueRefreshOutcome {
        let status = match discover_once(&self.base_dir, source) {
            Ok(models) => CatalogueRefreshStatus::Refreshed { models },
            Err(DiscoveryError::NotDiscoverable(reason)) => {
                CatalogueRefreshStatus::Skipped { reason }
            }
            Err(DiscoveryError::Failed(error)) => CatalogueRefreshStatus::Failed { error },
        };
        CatalogueRefreshOutcome {
            source: source.to_string(),
            status,
        }
    }
}

fn refresh_all_from_models_json(
    port: &dyn CatalogueRefreshPort,
    path: &Path,
) -> Vec<CatalogueRefreshOutcome> {
    // No user catalogue file means there is nothing to refresh, which is the
    // ordinary state for a user on built-in models — not a failure.
    if !path.exists() {
        return Vec::new();
    }
    match provider_keys(path) {
        Ok(keys) => {
            let deadline =
                std::time::Instant::now() + ModelsJsonCatalogueRefreshAdapter::REFRESH_ALL_BUDGET;
            keys.iter()
                .map(|key| {
                    if std::time::Instant::now() >= deadline {
                        // Reported rather than silently dropped: the user can see
                        // which sources the budget did not reach.
                        return CatalogueRefreshOutcome {
                            source: key.clone(),
                            status: CatalogueRefreshStatus::Skipped {
                                reason: "catalogue refresh budget exhausted before this source"
                                    .to_string(),
                            },
                        };
                    }
                    port.refresh_source(key)
                })
                .collect()
        }
        Err(error) => vec![CatalogueRefreshOutcome {
            source: "models.json".to_string(),
            status: CatalogueRefreshStatus::Failed { error },
        }],
    }
}

/// Why one source produced no models. `NotDiscoverable` is an ordinary state —
/// the provider has nothing for discovery to query — and is reported as skipped
/// rather than failed. Typed so rewording a message cannot silently reclassify
/// an outcome.
#[derive(Debug)]
pub(crate) enum DiscoveryError {
    NotDiscoverable(String),
    Failed(String),
}

impl From<String> for DiscoveryError {
    /// Any error that is not explicitly an ordinary "nothing to discover" state
    /// is a failure worth reporting.
    fn from(error: String) -> Self {
        Self::Failed(error)
    }
}

impl DiscoveryError {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn message(self) -> String {
        match self {
            Self::NotDiscoverable(message) | Self::Failed(message) => message,
        }
    }
}

pub(crate) fn discover_once(base_dir: &Path, provider_key: &str) -> Result<usize, DiscoveryError> {
    discover_once_with(
        base_dir,
        provider_key,
        fetch_openai_models,
        |path, bytes| atomic_write(path, bytes, Some(0o600)).map_err(|e| e.to_string()),
    )
}

pub(crate) fn discover_once_with<F, W>(
    base_dir: &Path,
    provider_key: &str,
    fetch: F,
    publish: W,
) -> Result<usize, DiscoveryError>
where
    F: FnOnce(&str, Option<&str>) -> Result<Vec<Value>, String>,
    W: FnOnce(&Path, &[u8]) -> Result<(), String>,
{
    let path = base_dir.join("models.json");
    let registry = read_registry(&path)?;
    let provider = provider_object(&registry, provider_key, &path)?;
    let initial_provider = Value::Object(provider.clone());
    let api = provider_api(provider, provider_key)?;
    if api != "openai-completions" {
        return Err(DiscoveryError::NotDiscoverable(format!(
            "provider '{provider_key}' is not an openai-completions provider"
        )));
    }
    let base_url = provider
        .get("baseUrl")
        .or_else(|| provider.get("apiBase"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            DiscoveryError::NotDiscoverable(format!(
                "provider '{provider_key}' has no baseUrl to query"
            ))
        })?;
    let allow_remote_http = provider
        .get("allowRemoteHttp")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    // Asked before the URL policy: an OAuth provider is unsupported by discovery
    // whatever its base URL looks like, and must be skipped rather than failed.
    if provider
        .get("auth")
        .and_then(|auth| auth.get("mode"))
        .and_then(Value::as_str)
        .is_some_and(|mode| mode == "oauth")
    {
        return Err(DiscoveryError::NotDiscoverable(format!(
            "provider '{provider_key}' uses oauth auth, which models discover does not support"
        )));
    }
    let url = discover_models_url(provider_key, base_url, allow_remote_http)?;
    let auth = provider
        .get("auth")
        .and_then(|auth| auth.get("apiKey"))
        .or_else(|| provider.get("apiKey"))
        .and_then(Value::as_str)
        .map(|value| resolve_registry_value(value, |name| std::env::var(name).ok()));
    let discovered = fetch(&url, auth.as_deref())?;
    let count = discovered.len();

    // Fetching may take seconds. Serialize the read-modify-write publication so
    // concurrent refreshes of different providers cannot both re-read the same
    // pre-publication file and lose whichever update writes first.
    let _publish_guard = ModelsJsonPublishLock::acquire(base_dir)?;
    let mut latest = read_registry(&path)?;
    let latest_provider = provider_object(&latest, provider_key, &path)?;
    let latest_provider_without_models = provider_without_models(latest_provider);
    if latest_provider_without_models != initial_provider_without_models(&initial_provider) {
        return Err(DiscoveryError::Failed(format!(
            "provider '{provider_key}' changed during discovery; discarding stale catalogue refresh"
        )));
    }
    provider_object_mut(&mut latest, provider_key, &path)?
        .insert("models".to_string(), Value::Array(discovered));
    let bytes = serde_json::to_vec_pretty(&latest)
        .map_err(|e| format!("failed to serialize registry: {e}"))?;
    serde_json::from_slice::<Value>(&bytes)
        .map_err(|e| format!("serialized registry was invalid JSON: {e}"))?;
    publish(&path, &bytes)
        .map_err(|e| format!("failed to write {} atomically: {e}", path.display()))?;
    Ok(count)
}

pub(crate) fn provider_keys(path: &Path) -> Result<Vec<String>, String> {
    let registry = read_registry(path)?;
    let providers = registry
        .get("providers")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            format!(
                "models registry {} is missing providers object",
                path.display()
            )
        })?;
    let mut keys: Vec<_> = providers.keys().cloned().collect();
    keys.sort();
    Ok(keys)
}

fn initial_provider_without_models(provider: &Value) -> Value {
    provider
        .as_object()
        .map(provider_without_models)
        .unwrap_or_else(|| provider.clone())
}

fn provider_without_models(provider: &serde_json::Map<String, Value>) -> Value {
    let mut comparable = provider.clone();
    comparable.remove("models");
    Value::Object(comparable)
}

fn read_registry(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("failed to parse {}: {e}", path.display()))
}

fn provider_api<'a>(
    provider: &'a serde_json::Map<String, Value>,
    provider_key: &str,
) -> Result<&'a str, String> {
    match provider.get("api") {
        None | Some(Value::Null) => Ok("openai-completions"),
        Some(Value::String(api)) => Ok(api),
        Some(_) => Err(format!("provider '{provider_key}' api must be a string")),
    }
}

fn provider_object<'a>(
    registry: &'a Value,
    provider_key: &str,
    path: &Path,
) -> Result<&'a serde_json::Map<String, Value>, String> {
    registry
        .get("providers")
        .and_then(Value::as_object)
        .and_then(|providers| providers.get(provider_key))
        .and_then(Value::as_object)
        .ok_or_else(|| format!("provider '{provider_key}' not found in {}", path.display()))
}

fn provider_object_mut<'a>(
    registry: &'a mut Value,
    provider_key: &str,
    path: &Path,
) -> Result<&'a mut serde_json::Map<String, Value>, String> {
    registry
        .get_mut("providers")
        .and_then(Value::as_object_mut)
        .and_then(|providers| providers.get_mut(provider_key))
        .and_then(Value::as_object_mut)
        .ok_or_else(|| format!("provider '{provider_key}' not found in {}", path.display()))
}

pub(crate) fn discover_models_url(
    provider_key: &str,
    base_url: &str,
    allow_remote_http: bool,
) -> Result<String, String> {
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
    Ok(models_url.to_string())
}

/// Discovery's name for the shared error-URL sanitiser.
pub(crate) fn redact_url_for_error(url: &str) -> String {
    crate::infrastructure::providers::sanitize_url_for_error(url)
}

pub(crate) fn format_reqwest_error(display_url: &str, e: reqwest::Error) -> String {
    let without_url = e.without_url();
    format!("GET {display_url} failed: {without_url}")
}

pub(crate) fn fetch_openai_models(url: &str, auth: Option<&str>) -> Result<Vec<Value>, String> {
    let display_url = redact_url_for_error(url);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;
    let mut req = client.get(url);
    if let Some(token) = auth.filter(|s| !s.is_empty()) {
        req = req.bearer_auth(token);
    }
    let resp = req
        .send()
        .map_err(|e| format_reqwest_error(&display_url, e))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("GET {display_url} returned {status}"));
    }
    let mut capped = resp.take((MAX_MODEL_DISCOVERY_RESPONSE_BYTES + 1) as u64);
    let mut bytes = Vec::new();
    capped
        .read_to_end(&mut bytes)
        .map_err(|e| format!("GET {display_url} failed while reading response: {e}"))?;
    if bytes.len() > MAX_MODEL_DISCOVERY_RESPONSE_BYTES {
        return Err(format!(
            "GET {display_url} response body exceeds {MAX_MODEL_DISCOVERY_RESPONSE_BYTES} bytes"
        ));
    }
    let body: Value = serde_json::from_slice(&bytes)
        .map_err(|e| format!("GET {display_url} returned invalid JSON: {e}"))?;
    let data = body
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| "model list response missing data array".to_string())?;
    if data.len() > MAX_MODEL_DISCOVERY_MODELS {
        return Err(format!(
            "model catalog contains more than {MAX_MODEL_DISCOVERY_MODELS} entries"
        ));
    }
    let mut models_by_id = HashMap::with_capacity(data.len());
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
        models_by_id.insert(id.to_string(), json!({ "id": id, "name": name }));
    }
    let mut models: Vec<_> = models_by_id.into_values().collect();
    models.sort_unstable_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
    Ok(models)
}

#[cfg(test)]
#[path = "catalogue_discovery_tests.rs"]
mod tests;
