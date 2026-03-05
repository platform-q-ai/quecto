//! Import credentials from opencode's auth.json file.

use super::Output;
use crate::infrastructure::auth::credential_store::{AuthMethod, Credential, CredentialStore};

/// Load and parse opencode's auth.json file.
pub(super) fn load_opencode_auth_json(stderr: &mut String) -> Option<serde_json::Value> {
    let auth_path = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("~/.local/share"))
        .join("opencode")
        .join("auth.json");

    if !auth_path.exists() {
        stderr.push_str(&format!(
            "auth login: opencode auth.json not found at {}\n",
            auth_path.display()
        ));
        return None;
    }

    let data = match std::fs::read_to_string(&auth_path) {
        Ok(d) => d,
        Err(e) => {
            stderr.push_str(&format!(
                "auth login: failed to read {}: {}\n",
                auth_path.display(),
                e
            ));
            return None;
        }
    };

    match serde_json::from_str(&data) {
        Ok(v) => Some(v),
        Err(e) => {
            stderr.push_str(&format!("auth login: failed to parse auth.json: {}\n", e));
            None
        }
    }
}

/// Import Anthropic OAuth credential from opencode auth.json.
pub(super) fn import_anthropic(
    auth_json: &serde_json::Value,
    store: &CredentialStore,
    rt: &tokio::runtime::Runtime,
    out: &mut Output<'_>,
) -> Option<u32> {
    let anthropic = match auth_json.get("anthropic") {
        Some(v) if v.get("type").and_then(|t| t.as_str()) == Some("oauth") => v,
        _ => return Some(0),
    };

    let refresh = anthropic
        .get("refresh")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let access = anthropic
        .get("access")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let expires_s = anthropic
        .get("expires")
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
        / 1000;
    let now = chrono::Utc::now().timestamp();

    let (token, refresh_tok, expires) = if now >= expires_s && !refresh.is_empty() {
        out.stdout
            .push_str("Anthropic token expired, refreshing...\n");
        let config = crate::infrastructure::auth::oauth::OAuthConfig::for_provider("anthropic");
        let Some(config) = config else {
            out.stderr
                .push_str("auth login: no Anthropic OAuth config\n");
            return None;
        };
        match rt.block_on(crate::infrastructure::auth::oauth::refresh_anthropic_token(
            &config, refresh,
        )) {
            Ok(resp) => (
                resp.access_token,
                resp.refresh_token.unwrap_or_else(|| refresh.to_string()),
                now + resp.expires_in as i64 - 300,
            ),
            Err(e) => {
                out.stderr.push_str(&format!(
                    "auth login: failed to refresh Anthropic token: {}\n",
                    e
                ));
                return None;
            }
        }
    } else {
        (access.to_string(), refresh.to_string(), expires_s - 300)
    };

    match store.store(Credential {
        provider: "anthropic".to_string(),
        token,
        method: AuthMethod::OAuth,
        expires_at: Some(expires),
        refresh_token: Some(refresh_tok),
        account_id: None,
    }) {
        Ok(()) => {
            out.stdout.push_str("Imported Anthropic OAuth credential\n");
            Some(1)
        }
        Err(e) => {
            out.stderr.push_str(&format!(
                "auth login: failed to store Anthropic credential: {}\n",
                e
            ));
            Some(0)
        }
    }
}

/// Import OpenAI OAuth credential from opencode auth.json.
pub(super) fn import_openai(
    auth_json: &serde_json::Value,
    store: &CredentialStore,
    out: &mut Output<'_>,
) -> u32 {
    let openai = match auth_json.get("openai") {
        Some(v) if v.get("type").and_then(|t| t.as_str()) == Some("oauth") => v,
        _ => return 0,
    };

    let access = openai.get("access").and_then(|v| v.as_str()).unwrap_or("");
    let refresh = openai.get("refresh").and_then(|v| v.as_str()).unwrap_or("");
    let expires_s = openai.get("expires").and_then(|v| v.as_i64()).unwrap_or(0) / 1000;

    if access.is_empty() {
        return 0;
    }

    let refresh_token = if refresh.is_empty() {
        None
    } else {
        Some(refresh.to_string())
    };

    match store.store(Credential {
        provider: "openai".to_string(),
        token: access.to_string(),
        method: AuthMethod::OAuth,
        expires_at: Some(expires_s),
        refresh_token,
        account_id: None,
    }) {
        Ok(()) => {
            out.stdout.push_str("Imported OpenAI OAuth credential\n");
            1
        }
        Err(e) => {
            out.stderr.push_str(&format!(
                "auth login: failed to store OpenAI credential: {}\n",
                e
            ));
            0
        }
    }
}
