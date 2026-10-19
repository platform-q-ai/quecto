#[path = "auth_import.rs"]
pub(crate) mod auth_import;
mod browser;
mod handles;
pub use handles::{AuthEnvironment, AuthHandles, AuthHandlesBuilder};

use super::CliContext;
use crate::application::auth::dto::LoginError;
use crate::domain::auth::provider::LoginProvider;
use crate::infrastructure::auth::credential_store::CredentialStore;

/// The login allowlist as a refusal lists it: "openai, anthropic, xai".
fn known_providers() -> String {
    LoginProvider::ALL.map(LoginProvider::id).join(", ")
}

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
    let mut input = ctx.stdin_data.as_deref().unwrap_or("").as_bytes();
    if ctx.stdin_data.is_some() {
        return cmd_auth_with_reader(ctx, args, stdout, stderr, &mut input);
    }
    let stdin = std::io::stdin();
    cmd_auth_with_reader(ctx, args, stdout, stderr, &mut stdin.lock())
}
pub(crate) fn cmd_auth_with_reader(
    ctx: &CliContext,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
    reader: &mut dyn std::io::BufRead,
) -> i32 {
    let base = ctx.base_dir();
    if args.is_empty() {
        stderr.push_str("auth: missing subcommand (login, logout, status)\n");
        return 1;
    }
    match args[0].as_str() {
        "login" => cmd_auth_login(ctx, &args[1..], stdout, stderr, reader),
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
    reader: &mut dyn std::io::BufRead,
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

    let provider = match resolve_provider_interactive(ctx, provider, &mut out, reader) {
        Some(p) => p,
        None => return 1,
    };

    browser::cmd_auth_login_oauth(ctx, &provider, &mut out, reader)
}

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
    let Some(handles) = handles::login_handles(ctx, out.stderr) else {
        return 1;
    };
    match handles.token_login.execute(&provider, token_val) {
        Ok(stored) => {
            out.stdout
                .push_str(&format!("Credential stored for {}\n", stored.id()));
            0
        }
        Err(error) => {
            out.stderr.push_str(&login_refusal(&error));
            1
        }
    }
}

/// A login's refusal as `auth login` prints it, newline-terminated.
pub(super) fn login_refusal(error: &LoginError) -> String {
    match error {
        LoginError::UnknownProvider(provider) => format!(
            "auth login: unknown provider '{}'. Known: {}\n",
            provider,
            known_providers()
        ),
        LoginError::EmptyToken => "auth login: --token value must not be empty\n".to_string(),
        LoginError::DeviceCodeUnsupported(provider) => format!(
            "auth login: device code flow is not supported for '{}' (use --oauth instead)\n",
            provider.id()
        ),
        LoginError::Unavailable(why) => format!("auth login: failed to create runtime: {why}\n"),
        LoginError::InsideAsyncRuntime => {
            "auth login: cannot start a login from inside an async runtime\n".to_string()
        }
        LoginError::DeviceCodeRequest(why) => {
            format!("auth login: device code request failed: {why}\n")
        }
        LoginError::OAuthUnsupported(provider) => format!(
            "auth login: OAuth is not supported for '{}'\n",
            provider.id()
        ),
        LoginError::Paste(refusal) => format!("auth login: {}\n", refusal.reason()),
        LoginError::EmptyAuthorizationCode => {
            "auth login: authorization code must not be empty\n".to_string()
        }
        LoginError::InvalidRedirect(why) => format!("auth login: invalid redirect URI: {why}\n"),
        LoginError::TokenExchange(why) => format!("auth login: token exchange failed: {why}\n"),
        LoginError::Store(why) => format!("auth login: failed to store credential: {why}\n"),
    }
}

fn resolve_provider_interactive(
    ctx: &CliContext,
    provider: Option<String>,
    out: &mut Output<'_>,
    reader: &mut dyn std::io::BufRead,
) -> Option<String> {
    match provider {
        Some(p) => {
            if LoginProvider::parse(&p).is_none() {
                out.stderr
                    .push_str(&login_refusal(&LoginError::UnknownProvider(p)));
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
            let choice = match read_line(reader) {
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

pub(super) fn read_line(reader: &mut dyn std::io::BufRead) -> Result<String, String> {
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|e| format!("failed to read from stdin: {e}"))?;
    Ok(line)
}

#[cfg(test)]
pub(crate) fn read_stdin_line(ctx: &CliContext) -> Result<String, String> {
    let data = ctx.stdin_data.as_deref().unwrap_or("");
    Ok(data.lines().next().unwrap_or("").to_string())
}

pub(crate) fn flush_stdout(ctx: &CliContext, out: &mut Output<'_>) {
    if ctx.stdin_data.is_some() || out.stdout.is_empty() {
        return;
    }
    use std::io::Write;
    print!("{}", out.stdout);
    let _ = std::io::stdout().flush();
    out.stdout.clear();
}

/// Redirected stdin only disables browser callbacks in the REPL, where waiting
/// would stall a command stream. Standalone callbacks do not require stdin.
pub(super) fn skip_browser_callback(ctx: &CliContext) -> bool {
    ctx.stdin_data.is_some() || (ctx.is_repl && ctx.stdin_is_tty == Some(false))
}

/// Import credentials from an external auth.json file.
fn cmd_auth_import_external(ctx: &CliContext, out: &mut Output<'_>) -> i32 {
    let auth_json = match auth_import::load_external_auth_json(out.stderr) {
        Some(v) => v,
        None => return 1,
    };

    let store = CredentialStore::new(ctx.base_dir());
    let rt = match crate::infrastructure::processes::tokio_runtime::build_tokio_runtime() {
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
    let Some(handles) = handles::login_handles(ctx, out.stderr) else {
        return 1;
    };
    match handles.device_code_login.execute(provider) {
        Ok(shown) => {
            out.stdout.push_str(&format!(
                "Go to: {}\nEnter code: {}\n\nWaiting for authorization...\n",
                shown.verification_uri, shown.user_code
            ));
            0
        }
        // A device-code login names no provider outside the allowlist as
        // unknown: it has no flow to offer one.
        Err(LoginError::UnknownProvider(provider)) => {
            out.stderr.push_str(&format!(
                "auth login: device code flow is not supported for '{}'\n",
                provider
            ));
            1
        }
        Err(error) => {
            out.stderr.push_str(&login_refusal(&error));
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

#[cfg(test)]
#[path = "auth_refusal_tests.rs"]
mod refusal_tests;
