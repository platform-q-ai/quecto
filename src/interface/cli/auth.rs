use super::CliContext;
use crate::infrastructure::auth::credential_store::{AuthMethod, Credential, CredentialStore};

/// Known provider names accepted by the auth commands.
const KNOWN_PROVIDERS: &[&str] = &["openai", "anthropic"];

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
    let base = ctx.base_dir();
    let mut provider: Option<String> = None;
    let mut token: Option<String> = None;
    let mut use_oauth = false;
    let mut use_device_code = false;
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
            "--oauth" => {
                use_oauth = true;
                i += 1;
            }
            "--device-code" => {
                use_device_code = true;
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

    let Some(provider) = provider else {
        stderr.push_str("auth login: --provider is required\n");
        return 1;
    };

    if !KNOWN_PROVIDERS.contains(&provider.as_str()) {
        stderr.push_str(&format!(
            "auth login: unknown provider '{}'. Known: {}\n",
            provider,
            KNOWN_PROVIDERS.join(", ")
        ));
        return 1;
    }

    if use_oauth {
        return cmd_auth_login_oauth(ctx, &provider, stdout, stderr);
    }

    if use_device_code {
        return cmd_auth_login_device_code(ctx, &provider, stdout, stderr);
    }

    // If --token was provided, use it directly.
    // Otherwise, prompt for interactive token paste.
    let token = match token {
        Some(t) => t,
        None => {
            stdout.push_str(&format!("Paste your API token for {}:\n", provider));
            match read_stdin_line(ctx) {
                Ok(line) => line,
                Err(e) => {
                    stderr.push_str(&format!("auth login: {}\n", e));
                    return 1;
                }
            }
        }
    };

    let token = token.trim().to_string();
    if token.is_empty() {
        stderr.push_str("auth login: --token value must not be empty\n");
        return 1;
    }

    let store = CredentialStore::new(&base);
    match store.store(Credential {
        provider: provider.clone(),
        token,
        method: AuthMethod::Token,
        expires_at: None,
    }) {
        Ok(()) => {
            stdout.push_str(&format!("Credential stored for {}\n", provider));
            0
        }
        Err(e) => {
            stderr.push_str(&format!("auth login: failed to store credential: {}\n", e));
            1
        }
    }
}

/// Read a single line from stdin (or from `ctx.stdin_data` in test mode).
/// Returns `Err` with an error message if stdin cannot be read.
pub(crate) fn read_stdin_line(ctx: &CliContext) -> Result<String, String> {
    if let Some(ref data) = ctx.stdin_data {
        // Return the first line of pre-loaded stdin data.
        Ok(data.lines().next().unwrap_or("").to_string())
    } else {
        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .map_err(|e| format!("failed to read from stdin: {}", e))?;
        Ok(line)
    }
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
fn cmd_auth_login_oauth(
    ctx: &CliContext,
    provider: &str,
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let config = match resolve_oauth_config(ctx, provider, "OAuth", stderr) {
        Some(c) => c,
        None => return 1,
    };

    stdout.push_str(&format!(
        "Open this URL in your browser:\n{}\n\nWaiting for authorization...\n",
        config.authorization_url
    ));
    0
}

/// Device code login flow for headless environments.
fn cmd_auth_login_device_code(
    ctx: &CliContext,
    provider: &str,
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let config = match resolve_oauth_config(ctx, provider, "device code flow", stderr) {
        Some(c) => c,
        None => return 1,
    };

    let rt = match super::build_tokio_runtime() {
        Ok(rt) => rt,
        Err(e) => {
            stderr.push_str(&format!("auth login: failed to create runtime: {}\n", e));
            return 1;
        }
    };
    match rt.block_on(crate::infrastructure::auth::oauth::request_device_code(
        &config,
    )) {
        Ok(resp) => {
            stdout.push_str(&format!(
                "Go to: {}\nEnter code: {}\n\nWaiting for authorization...\n",
                resp.verification_uri, resp.user_code
            ));
            0
        }
        Err(e) => {
            stderr.push_str(&format!("auth login: device code request failed: {}\n", e));
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
