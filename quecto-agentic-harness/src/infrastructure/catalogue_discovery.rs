//! Catalogue refresh/discovery infrastructure adapters.
//!
//! Network discovery, provider-specific URL validation, response bounds, and
//! `models.json` publication are infrastructure concerns. Interfaces call the
//! application refresh use case with this adapter rather than owning discovery
//! behaviour themselves.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use crate::catalogue_refresh_app::{
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
}

impl CatalogueRefreshAllPort for ModelsJsonCatalogueRefreshAdapter {
    fn refresh_all_sources(&self) -> Vec<CatalogueRefreshOutcome> {
        self.refresh_all()
    }
}

impl CatalogueRefreshPort for ModelsJsonCatalogueRefreshAdapter {
    fn refresh_source(&self, source: &str) -> CatalogueRefreshOutcome {
        match discover_once(&self.base_dir, source) {
            Ok(models) => CatalogueRefreshOutcome {
                source: source.to_string(),
                status: CatalogueRefreshStatus::Refreshed { models },
            },
            Err(error) if is_unsupported_refresh_error(&error) => CatalogueRefreshOutcome {
                source: source.to_string(),
                status: CatalogueRefreshStatus::Skipped { reason: error },
            },
            Err(error) => CatalogueRefreshOutcome {
                source: source.to_string(),
                status: CatalogueRefreshStatus::Failed { error },
            },
        }
    }
}

fn refresh_all_from_models_json(
    port: &dyn CatalogueRefreshPort,
    path: &Path,
) -> Vec<CatalogueRefreshOutcome> {
    match provider_keys(path) {
        Ok(keys) => keys.iter().map(|key| port.refresh_source(key)).collect(),
        Err(error) => vec![CatalogueRefreshOutcome {
            source: "models.json".to_string(),
            status: CatalogueRefreshStatus::Failed { error },
        }],
    }
}

fn is_unsupported_refresh_error(error: &str) -> bool {
    error.contains("is not an openai-completions provider")
        || error.contains("uses oauth auth, which models discover does not support")
}

pub(crate) fn discover_once(base_dir: &Path, provider_key: &str) -> Result<usize, String> {
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
) -> Result<usize, String>
where
    F: FnOnce(&str, Option<&str>) -> Result<Vec<Value>, String>,
    W: FnOnce(&Path, &[u8]) -> Result<(), String>,
{
    let path = base_dir.join("models.json");
    let registry = read_registry(&path)?;
    let provider = provider_object(&registry, provider_key, &path)?;
    let api = provider_api(provider, provider_key)?;
    if api != "openai-completions" {
        return Err(format!(
            "provider '{provider_key}' is not an openai-completions provider"
        ));
    }
    let base_url = provider
        .get("baseUrl")
        .or_else(|| provider.get("apiBase"))
        .and_then(Value::as_str)
        .ok_or_else(|| format!("provider '{provider_key}' is missing baseUrl"))?;
    let allow_remote_http = provider
        .get("allowRemoteHttp")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let url = discover_models_url(provider_key, base_url, allow_remote_http)?;
    if provider
        .get("auth")
        .and_then(|auth| auth.get("mode"))
        .and_then(Value::as_str)
        .is_some_and(|mode| mode == "oauth")
    {
        return Err(format!(
            "provider '{provider_key}' uses oauth auth, which models discover does not support"
        ));
    }
    let auth = provider
        .get("auth")
        .and_then(|auth| auth.get("apiKey"))
        .or_else(|| provider.get("apiKey"))
        .and_then(Value::as_str)
        .map(|value| resolve_registry_value(value, |name| std::env::var(name).ok()));
    let discovered = fetch(&url, auth.as_deref())?;
    let count = discovered.len();

    // Fetching may take seconds. Re-read immediately before the whole-file
    // publication so unrelated providers changed meanwhile are preserved.
    let mut latest = read_registry(&path)?;
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

pub(crate) fn redact_url_for_error(url: &str) -> String {
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
