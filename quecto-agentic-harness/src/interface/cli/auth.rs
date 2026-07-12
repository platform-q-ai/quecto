#[path = "auth_import.rs"]
pub(crate) mod auth_import;

#[path = "auth_xai.rs"]
pub(crate) mod auth_xai;

use super::CliContext;
use crate::infrastructure::auth::credential_store::{AuthMethod, Credential, CredentialStore};

/// Known provider names accepted by the auth commands.
const KNOWN_PROVIDERS: &[&str] = &["openai", "anthropic", "xai"];

/// Bundled output streams for auth subcommands.
pub(crate) struct Output<'a> {
    pub stdout: &'a mut String,
    pub stderr: &'a mut String,
}

pub(crate) fn cmd_auth(
    ctx: &CliContext,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let base = ctx.base_dir();

    if args.is_empty() {
        stderr.push_str("auth: missing subcommand (login, logout, status)\n");
        return 1;
    }

    match args[0].as_str() {
        "login" => cmd_auth_login(ctx, &args[1..], stdout, stderr),
        "logout" => cmd_auth_logout(&base, &args[1..], stdout, stderr),
        "status" => cmd_auth_status(&base, stdout),
        other => {
            stderr.push_str(&format!("auth: unknown subcommand '{}'\n", other));
            1
        }
    }
}

fn cmd_auth_login(
    ctx: &CliContext,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let mut provider: Option<String> = None;
    let mut token: Option<String> = None;
    let mut use_device_code = false;
    let mut import_external = false;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--provider" => {
                if i + 1 < args.len() {
                    provider = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    stderr.push_str("auth login: --provider requires a value\n");
                    return 1;
                }
            }
            "--token" => {
                if i + 1 < args.len() {
                    token = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    stderr.push_str("auth login: --token requires a value\n");
                    return 1;
                }
            }
            "--device-code" => {
                use_device_code = true;
                i += 1;
            }
            "--import-external" => {
                import_external = true;
                i += 1;
            }
            // Keep --oauth as a silent alias (backwards compat)
            "--oauth" => {
                i += 1;
            }
            other if other.starts_with("--") => {
                stderr.push_str(&format!("auth login: unknown flag '{}'\n", other));
                return 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    let mut out = Output { stdout, stderr };

    if import_external {
        return cmd_auth_import_external(ctx, &mut out);
    }

    if let Some(token_val) = token {
        return cmd_auth_login_token(ctx, provider, &token_val, &mut out);
    }

    if use_device_code {
        let Some(provider) = provider else {
            out.stderr
                .push_str("auth login: --provider is required when using --device-code\n");
            return 1;
        };
        return cmd_auth_login_device_code(ctx, &provider, &mut out);
    }

    let provider = match resolve_provider_interactive(ctx, provider, &mut out) {
        Some(p) => p,
        None => return 1,
    };

    cmd_auth_login_oauth(ctx, &provider, &mut out)
}

/// Handle `--token` direct API key login.
fn cmd_auth_login_token(
    ctx: &CliContext,
    provider: Option<String>,
    token_val: &str,
    out: &mut Output<'_>,
) -> i32 {
    let Some(provider) = provider else {
        out.stderr
            .push_str("auth login: --provider is required when using --token\n");
        return 1;
    };
    if !KNOWN_PROVIDERS.contains(&provider.as_str()) {
        out.stderr.push_str(&format!(
            "auth login: unknown provider '{}'. Known: {}\n",
            provider,
            KNOWN_PROVIDERS.join(", ")
        ));
        return 1;
    }
    let token = token_val.trim().to_string();
    if token.is_empty() {
        out.stderr
            .push_str("auth login: --token value must not be empty\n");
        return 1;
    }
    let store = CredentialStore::new(ctx.base_dir());
    match store.store(Credential {
        provider: provider.clone(),
        token,
        method: AuthMethod::Token,
        expires_at: None,
        refresh_token: None,
        account_id: None,
    }) {
        Ok(()) => {
            out.stdout
                .push_str(&format!("Credential stored for {}\n", provider));
            0
        }
        Err(e) => {
            out.stderr
                .push_str(&format!("auth login: failed to store credential: {}\n", e));
            1
        }
    }
}

/// Resolve the provider interactively if not specified, or validate the given one.
fn resolve_provider_interactive(
    ctx: &CliContext,
    provider: Option<String>,
    out: &mut Output<'_>,
) -> Option<String> {
    match provider {
        Some(p) => {
            if !KNOWN_PROVIDERS.contains(&p.as_str()) {
                out.stderr.push_str(&format!(
                    "auth login: unknown provider '{}'. Known: {}\n",
                    p,
                    KNOWN_PROVIDERS.join(", ")
                ));
                return None;
            }
            Some(p)
        }
        None => {
            out.stdout.push_str(
                "Choose a provider:\n  1) Anthropic (Claude Pro/Max — OAuth)\n  \
                 2) OpenAI (OAuth)\n  3) xAI (SuperGrok / X Premium+ — OAuth)\n\nEnter 1, 2 or 3: ",
            );
            flush_stdout(ctx, out);
            let choice = match read_stdin_line(ctx) {
                Ok(line) => line.trim().to_string(),
                Err(e) => {
                    out.stderr.push_str(&format!("auth login: {}\n", e));
                    return None;
                }
            };
            match choice.as_str() {
                "1" | "anthropic" => Some("anthropic".to_string()),
                "2" | "openai" => Some("openai".to_string()),
                "3" | "xai" | "grok" => Some("xai".to_string()),
                _ => {
                    out.stderr
                        .push_str(&format!("auth login: invalid choice '{}'\n", choice));
                    None
                }
            }
        }
    }
}

/// Read a single line from stdin (or from `ctx.stdin_data` in test mode).
pub(crate) fn read_stdin_line(ctx: &CliContext) -> Result<String, String> {
    if let Some(ref data) = ctx.stdin_data {
        Ok(data.lines().next().unwrap_or("").to_string())
    } else {
        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .map_err(|e| format!("failed to read from stdin: {}", e))?;
        Ok(line)
    }
}

/// Flush buffered stdout text to the terminal immediately (for interactive prompts).
/// In test mode (when `stdin_data` is set), we skip the flush to preserve output
/// in the buffer for assertions.
pub(crate) fn flush_stdout(ctx: &CliContext, out: &mut Output<'_>) {
    if ctx.stdin_data.is_some() || out.stdout.is_empty() {
        return;
    }
    use std::io::Write;
    print!("{}", out.stdout);
    let _ = std::io::stdout().flush();
    out.stdout.clear();
}

/// Resolve OAuth config: use test override if set, otherwise look up the provider.
fn resolve_oauth_config(
    ctx: &CliContext,
    provider: &str,
    flow_name: &str,
    stderr: &mut String,
) -> Option<crate::infrastructure::auth::oauth::OAuthConfig> {
    use crate::infrastructure::auth::oauth::OAuthConfig;

    if let Some(ref base_url) = ctx.oauth_base_url {
        Some(OAuthConfig::with_base_url(base_url))
    } else {
        match OAuthConfig::for_provider(provider) {
            Some(c) => Some(c),
            None => {
                stderr.push_str(&format!(
                    "auth login: {} is not supported for '{}'\n",
                    flow_name, provider
                ));
                None
            }
        }
    }
}

/// OAuth browser-based login flow.
fn cmd_auth_login_oauth(ctx: &CliContext, provider: &str, out: &mut Output<'_>) -> i32 {
    let config = match resolve_oauth_config(ctx, provider, "OAuth", out.stderr) {
        Some(c) => c,
        None => return 1,
    };

    if provider == "anthropic" {
        return cmd_auth_login_anthropic_oauth(ctx, &config, out);
    }

    if provider == "openai" {
        return cmd_auth_login_openai_oauth(ctx, &config, out);
    }

    if provider == "xai" {
        return auth_xai::cmd_auth_login_xai_oauth(ctx, &config, out);
    }

    out.stdout.push_str(&format!(
        "Open this URL in your browser:\n{}\n\nWaiting for authorization...\n",
        config.authorization_url
    ));
    0
}

/// OpenAI OAuth login: PKCE + browser callback on localhost:1455.
fn cmd_auth_login_openai_oauth(
    ctx: &CliContext,
    config: &crate::infrastructure::auth::oauth::OAuthConfig,
    out: &mut Output<'_>,
) -> i32 {
    use crate::infrastructure::auth::oauth::{
        build_openai_auth_url, exchange_openai_code, extract_openai_account_id, generate_pkce,
        generate_state, wait_for_oauth_callback,
    };

    let pkce = generate_pkce();
    let state = generate_state();
    let auth_url = build_openai_auth_url(config, &pkce, &state);

    out.stdout.push_str(&format!(
        "Open this URL in your browser to authenticate with OpenAI:\n\n{}\n\n\
         Waiting for browser callback on http://localhost:1455 ...\n\
         (If the browser doesn't open, copy the URL above and paste it manually)\n",
        auth_url
    ));
    flush_stdout(ctx, out);

    let rt = match super::build_tokio_runtime() {
        Ok(rt) => rt,
        Err(e) => {
            out.stderr
                .push_str(&format!("auth login: failed to create runtime: {}\n", e));
            return 1;
        }
    };

    // In test mode (stdin_data set), skip the browser callback and go
    // straight to the manual code-paste fallback.
    let code = if ctx.stdin_data.is_some() {
        let err = crate::domain::error::DomainError::Provider(
            "browser callback skipped in test mode".into(),
        );
        match extract_fallback_code(ctx, err, Some(&state), out) {
            Some(code) => code,
            None => return 1,
        }
    } else {
        match rt.block_on(wait_for_oauth_callback(&state, 300)) {
            Ok(code) => code,
            Err(e) => match extract_fallback_code(ctx, e, Some(&state), out) {
                Some(code) => code,
                None => return 1,
            },
        }
    };

    match rt.block_on(exchange_openai_code(config, &code, &pkce.verifier)) {
        Ok(token_resp) => {
            let account_id = extract_openai_account_id(&token_resp.access_token);
            if account_id.is_none() {
                out.stderr
                    .push_str("auth login: warning — could not extract account ID from token\n");
            }
            let expires = crate::interface::shared::expires_at_with_margin(token_resp.expires_in);
            let params = OAuthStoreParams {
                provider: "openai".to_string(),
                account_id,
                expires_at: expires,
            };
            store_oauth_credential(ctx, params, &token_resp, out)
        }
        Err(e) => {
            out.stderr
                .push_str(&format!("auth login: token exchange failed: {}\n", e));
            1
        }
    }
}

/// Fallback: prompt user to paste code when callback fails.
///
/// When `expected_state` is provided and the pasted input is a redirect URL
/// or query fragment carrying a `state` parameter, the state must match or
/// the input is rejected (PR #1087 review). Bare authorization codes cannot
/// carry state and are accepted as-is (some providers display a bare code).
pub(crate) fn extract_fallback_code(
    ctx: &CliContext,
    err: crate::domain::error::DomainError,
    expected_state: Option<&str>,
    out: &mut Output<'_>,
) -> Option<String> {
    out.stdout.push_str(&format!(
        "\nCallback failed ({}). Paste the authorization code or redirect URL:\n",
        err
    ));
    flush_stdout(ctx, out);
    match read_stdin_line(ctx) {
        Ok(line) => {
            let line = line.trim().to_string();
            if let (Some(expected), Some(state)) =
                (expected_state, extract_param_from_input(&line, "state"))
            {
                if state != expected {
                    out.stderr
                        .push_str("auth login: state mismatch in pasted redirect URL\n");
                    return None;
                }
            }
            let code = extract_code_from_input(&line);
            if code.is_none() {
                out.stderr
                    .push_str("auth login: could not extract authorization code\n");
            }
            code
        }
        Err(e) => {
            out.stderr.push_str(&format!("auth login: {}\n", e));
            None
        }
    }
}

/// Parameters for storing an OAuth credential.
pub(crate) struct OAuthStoreParams {
    pub(crate) provider: String,
    pub(crate) account_id: Option<String>,
    pub(crate) expires_at: i64,
}

/// Store an OAuth credential after a successful token exchange.
pub(crate) fn store_oauth_credential(
    ctx: &CliContext,
    params: OAuthStoreParams,
    token_resp: &crate::infrastructure::auth::oauth::OAuthTokenResponse,
    out: &mut Output<'_>,
) -> i32 {
    let store = CredentialStore::new(ctx.base_dir());
    match store.store(Credential {
        provider: params.provider.clone(),
        token: token_resp.access_token.clone(),
        method: AuthMethod::OAuth,
        expires_at: Some(params.expires_at),
        refresh_token: token_resp.refresh_token.clone(),
        account_id: params.account_id,
    }) {
        Ok(()) => {
            out.stdout.push_str(&format!(
                "{} OAuth credential stored successfully\n",
                capitalize(&params.provider)
            ));
            0
        }
        Err(e) => {
            out.stderr
                .push_str(&format!("auth login: failed to store credential: {}\n", e));
            1
        }
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().to_string() + c.as_str(),
    }
}

/// Try to extract an authorization code from user input (URL or raw code).
fn extract_code_from_input(input: &str) -> Option<String> {
    // Try as URL: http://localhost:1455/auth/callback?code=<code>&state=<state>
    if input.contains("code=") {
        if let Some(code) = extract_param_from_input(input, "code") {
            return Some(code);
        }
    }
    if !input.is_empty() {
        Some(input.to_string())
    } else {
        None
    }
}

/// Extract a URL-decoded query parameter from a pasted URL or query string.
fn extract_param_from_input(input: &str, name: &str) -> Option<String> {
    let query = input.split('?').nth(1).unwrap_or(input);
    let prefix = format!("{}=", name);
    for param in query.split('&') {
        if let Some(value) = param.strip_prefix(prefix.as_str()) {
            if !value.is_empty() {
                return Some(
                    urlencoding::decode(value)
                        .map(|v| v.into_owned())
                        .unwrap_or_else(|_| value.to_string()),
                );
            }
        }
    }
    None
}

/// Anthropic OAuth login: PKCE + browser + paste authorization code.
fn cmd_auth_login_anthropic_oauth(
    ctx: &CliContext,
    config: &crate::infrastructure::auth::oauth::OAuthConfig,
    out: &mut Output<'_>,
) -> i32 {
    use crate::infrastructure::auth::oauth::{
        build_anthropic_auth_url, exchange_anthropic_code, generate_pkce, generate_state,
    };

    let pkce = generate_pkce();
    let state = generate_state();
    let auth_url = build_anthropic_auth_url(config, &pkce, &state);

    out.stdout.push_str(&format!(
        "Open this URL in your browser to authenticate with Anthropic:\n\n{}\n\n\
         After authorizing, you'll be redirected to a page with a URL containing\n\
         an authorization code. Copy the FULL URL or code and paste it below.\n\n\
         Paste the authorization code:\n",
        auth_url
    ));
    flush_stdout(ctx, out);

    let auth_code = match read_stdin_line(ctx) {
        Ok(line) => line.trim().to_string(),
        Err(e) => {
            out.stderr.push_str(&format!("auth login: {}\n", e));
            return 1;
        }
    };

    if auth_code.is_empty() {
        out.stderr
            .push_str("auth login: authorization code must not be empty\n");
        return 1;
    }

    let rt = match super::build_tokio_runtime() {
        Ok(rt) => rt,
        Err(e) => {
            out.stderr
                .push_str(&format!("auth login: failed to create runtime: {}\n", e));
            return 1;
        }
    };

    match rt.block_on(exchange_anthropic_code(config, &auth_code, &pkce.verifier)) {
        Ok(token_resp) => {
            let expires = crate::interface::shared::expires_at_with_margin(token_resp.expires_in);
            let params = OAuthStoreParams {
                provider: "anthropic".to_string(),
                account_id: None,
                expires_at: expires,
            };
            store_oauth_credential(ctx, params, &token_resp, out)
        }
        Err(e) => {
            out.stderr
                .push_str(&format!("auth login: token exchange failed: {}\n", e));
            1
        }
    }
}

/// Import credentials from an external auth.json file.
fn cmd_auth_import_external(ctx: &CliContext, out: &mut Output<'_>) -> i32 {
    let auth_json = match auth_import::load_external_auth_json(out.stderr) {
        Some(v) => v,
        None => return 1,
    };

    let store = CredentialStore::new(ctx.base_dir());
    let rt = match super::build_tokio_runtime() {
        Ok(rt) => rt,
        Err(e) => {
            out.stderr
                .push_str(&format!("auth login: failed to create runtime: {}\n", e));
            return 1;
        }
    };

    let mut imported = 0;

    match auth_import::import_anthropic(&auth_json, &store, &rt, out) {
        Some(n) => imported += n,
        None => return 1,
    }

    let openai_params = auth_import::OpenAiImportParams {
        store: &store,
        rt: &rt,
        oauth_base_url: None,
    };
    match auth_import::import_openai(&auth_json, &openai_params, out) {
        Some(n) => imported += n,
        None => return 1,
    }

    if imported == 0 {
        out.stderr
            .push_str("auth login: no OAuth credentials found in auth.json\n");
        return 1;
    }

    out.stdout
        .push_str(&format!("Imported {} credential(s)\n", imported));
    0
}

/// Device code login flow for headless environments.
fn cmd_auth_login_device_code(ctx: &CliContext, provider: &str, out: &mut Output<'_>) -> i32 {
    let config = match resolve_oauth_config(ctx, provider, "device code flow", out.stderr) {
        Some(c) => c,
        None => return 1,
    };

    if config.device_code_url.is_empty() {
        out.stderr.push_str(&format!(
            "auth login: device code flow is not supported for '{}' (use --oauth instead)\n",
            provider
        ));
        return 1;
    }

    let rt = match super::build_tokio_runtime() {
        Ok(rt) => rt,
        Err(e) => {
            out.stderr
                .push_str(&format!("auth login: failed to create runtime: {}\n", e));
            return 1;
        }
    };
    match rt.block_on(crate::infrastructure::auth::oauth::request_device_code(
        &config,
    )) {
        Ok(resp) => {
            out.stdout.push_str(&format!(
                "Go to: {}\nEnter code: {}\n\nWaiting for authorization...\n",
                resp.verification_uri, resp.user_code
            ));
            0
        }
        Err(e) => {
            out.stderr
                .push_str(&format!("auth login: device code request failed: {}\n", e));
            1
        }
    }
}

fn cmd_auth_logout(
    base: &std::path::Path,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let mut provider: Option<String> = None;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--provider" => {
                if i + 1 < args.len() {
                    provider = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    stderr.push_str("auth logout: --provider requires a value\n");
                    return 1;
                }
            }
            other if other.starts_with("--") => {
                stderr.push_str(&format!("auth logout: unknown flag '{}'\n", other));
                return 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    let Some(provider) = provider else {
        stderr.push_str("auth logout: --provider is required\n");
        return 1;
    };

    let store = CredentialStore::new(base);
    match store.remove(&provider) {
        Ok(true) => {
            stdout.push_str(&format!("Credential removed for {}\n", provider));
            0
        }
        Ok(false) => {
            stdout.push_str(&format!("no credential found for {}\n", provider));
            0
        }
        Err(e) => {
            stderr.push_str(&format!(
                "auth logout: failed to remove credential: {}\n",
                e
            ));
            1
        }
    }
}

fn cmd_auth_status(base: &std::path::Path, stdout: &mut String) -> i32 {
    let store = CredentialStore::new(base);
    match store.status_summary() {
        Ok(statuses) => {
            if statuses.is_empty() {
                stdout.push_str("no credentials stored\n");
            } else {
                stdout.push_str("Credentials:\n");
                for s in &statuses {
                    stdout.push_str(&format!("  {} ({}) — {}\n", s.provider, s.method, s.status));
                }
            }
            0
        }
        Err(e) => {
            stdout.push_str(&format!("failed to read credentials: {}\n", e));
            1
        }
    }
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "auth_cov_tests.rs"]
mod cov_tests;
