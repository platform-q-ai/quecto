use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};

use super::CliContext;
use crate::infrastructure::atomic_write::atomic_write;
use crate::infrastructure::model_registry::resolve_registry_value;
use crate::infrastructure::providers::{
    ProviderFactoryError, validate_provider_api_base_with_options,
};

pub fn cmd_models(
    ctx: &CliContext,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    match args.first().map(String::as_str) {
        Some("discover") => cmd_discover(ctx, &args[1..], stdout, stderr),
        _ => {
            stderr.push_str(
                "Usage: quecto models discover <provider-key> [--watch] [--interval <seconds>]\n",
            );
            1
        }
    }
}

fn cmd_discover(
    ctx: &CliContext,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    if args.is_empty() {
        stderr.push_str(
            "Usage: quecto models discover <provider-key> [--watch] [--interval <seconds>]\n",
        );
        return 1;
    }
    let provider = args[0].clone();
    let mut watch = false;
    let mut interval = 300_u64;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--watch" => {
                watch = true;
                i += 1;
            }
            "--interval" if i + 1 < args.len() => {
                interval = match args[i + 1].parse() {
                    Ok(0) => {
                        stderr.push_str("--interval must be at least 1 second\n");
                        return 1;
                    }
                    Ok(v) => v,
                    Err(_) => {
                        stderr.push_str("--interval must be an integer number of seconds\n");
                        return 1;
                    }
                };
                i += 2;
            }
            other => {
                stderr.push_str(&format!("Unknown models discover option: {other}\n"));
                return 1;
            }
        }
    }

    loop {
        match discover_once(ctx, &provider) {
            Ok(count) => stdout.push_str(&format!(
                "Discovered {count} model(s) for provider {provider}\n"
            )),
            Err(e) => {
                stderr.push_str(&format!("models discover failed: {e}\n"));
                return 1;
            }
        }
        if !watch {
            return 0;
        }
        std::thread::sleep(Duration::from_secs(interval));
    }
}

// Keep unattended discovery bounded: provider catalogs are small JSON lists,
// while compromised endpoints can otherwise stream arbitrary bytes/items.
const MAX_MODEL_DISCOVERY_RESPONSE_BYTES: usize = 5 * 1024 * 1024;
const MAX_MODEL_DISCOVERY_MODELS: usize = 10_000;

pub fn discover_once(ctx: &CliContext, provider_key: &str) -> Result<usize, String> {
    discover_once_with(ctx, provider_key, fetch_openai_models, |path, bytes| {
        atomic_write(path, bytes, Some(0o600)).map_err(|e| e.to_string())
    })
}

fn discover_once_with<F, W>(
    ctx: &CliContext,
    provider_key: &str,
    fetch: F,
    publish: W,
) -> Result<usize, String>
where
    F: FnOnce(&str, Option<&str>) -> Result<Vec<Value>, String>,
    W: FnOnce(&Path, &[u8]) -> Result<(), String>,
{
    let path = ctx.base_dir().join("models.json");
    let registry = read_registry(&path)?;
    let provider = provider_object(&registry, provider_key, &path)?;
    let api = match provider.get("api") {
        None | Some(Value::Null) => "openai-completions",
        Some(Value::String(api)) => api,
        Some(_) => return Err(format!("provider '{provider_key}' api must be a string")),
    };
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

fn read_registry(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("failed to parse {}: {e}", path.display()))
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

fn discover_models_url(
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

fn format_reqwest_error(display_url: &str, e: reqwest::Error) -> String {
    let without_url = e.without_url();
    format!("GET {display_url} failed: {without_url}")
}

fn fetch_openai_models(url: &str, auth: Option<&str>) -> Result<Vec<Value>, String> {
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
#[path = "models_tests.rs"]
mod tests;
