use std::collections::HashMap;
use std::time::Duration;

use serde_json::{Value, json};

use super::CliContext;
use crate::infrastructure::atomic_write::atomic_write;
use crate::infrastructure::model_registry::resolve_registry_value;

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

pub fn discover_once(ctx: &CliContext, provider_key: &str) -> Result<usize, String> {
    let path = ctx.base_dir().join("models.json");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    let mut registry: Value = serde_json::from_str(&text)
        .map_err(|e| format!("failed to parse {}: {e}", path.display()))?;
    let provider = registry
        .get_mut("providers")
        .and_then(Value::as_object_mut)
        .and_then(|providers| providers.get_mut(provider_key))
        .and_then(Value::as_object_mut)
        .ok_or_else(|| format!("provider '{provider_key}' not found in {}", path.display()))?;
    let api = provider
        .get("api")
        .and_then(Value::as_str)
        .unwrap_or("openai-completions");
    if api != "openai-completions" {
        return Err(format!(
            "provider '{provider_key}' is not an openai-completions provider"
        ));
    }
    let base_url = provider
        .get("baseUrl")
        .or_else(|| provider.get("api_base"))
        .and_then(Value::as_str)
        .ok_or_else(|| format!("provider '{provider_key}' is missing baseUrl"))?;
    let url = format!("{}/models", base_url.trim_end_matches('/'));
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
    let discovered = fetch_openai_models(&url, auth.as_deref())?;
    let count = discovered.len();
    let models = merge_discovered_models(provider.get("models"), discovered);
    provider.insert("models".to_string(), Value::Array(models));
    let bytes = serde_json::to_vec_pretty(&registry)
        .map_err(|e| format!("failed to serialize registry: {e}"))?;
    serde_json::from_slice::<Value>(&bytes)
        .map_err(|e| format!("serialized registry was invalid JSON: {e}"))?;
    atomic_write(&path, &bytes, Some(0o600))
        .map_err(|e| format!("failed to write {} atomically: {e}", path.display()))?;
    Ok(count)
}

fn merge_discovered_models(existing: Option<&Value>, discovered: Vec<Value>) -> Vec<Value> {
    let existing_by_id: HashMap<String, Value> = existing
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|model| {
            model
                .get("id")
                .and_then(Value::as_str)
                .map(|id| (id.to_string(), model.clone()))
        })
        .collect();

    discovered
        .into_iter()
        .map(|model| {
            let Some(id) = model.get("id").and_then(Value::as_str) else {
                return model;
            };
            let mut merged = existing_by_id
                .get(id)
                .cloned()
                .unwrap_or(Value::Object(Default::default()));
            if let Some(obj) = merged.as_object_mut() {
                obj.insert("id".to_string(), Value::String(id.to_string()));
                if let Some(name) = model.get("name") {
                    obj.insert("name".to_string(), name.clone());
                }
                merged
            } else {
                model
            }
        })
        .collect()
}

fn redact_url_for_error(url: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(mut parsed) => {
            let _ = parsed.set_username("");
            let _ = parsed.set_password(None);
            parsed.set_query(None);
            parsed.to_string()
        }
        Err(_) => "<invalid url>".to_string(),
    }
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
        .map_err(|e| format!("GET {display_url} failed: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("GET {display_url} returned {status}"));
    }
    let body: Value = resp
        .json()
        .map_err(|e| format!("GET {display_url} returned invalid JSON: {e}"))?;
    body.get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| "model list response missing data array".to_string())?
        .iter()
        .map(|m| {
            let id = m
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| "model entry missing string id".to_string())?;
            let name = m
                .get("name")
                .or_else(|| m.get("owned_by"))
                .and_then(Value::as_str)
                .unwrap_or(id);
            Ok(json!({ "id": id, "name": name }))
        })
        .collect()
}

#[cfg(test)]
#[path = "models_tests.rs"]
mod tests;
