//! Shared utility functions used by CLI, REPL, and gateway modules.

use std::collections::HashMap;
use std::path::Path;

use chrono::Local;

use crate::domain::skill::SkillLoader;
use crate::infrastructure::auth::credential_store::Credential;
use crate::infrastructure::persistence::skill_loader::FileSkillLoader;

/// Load all workspace skills and concatenate their non-empty body content.
///
/// Skills without valid YAML frontmatter are silently skipped.
pub fn load_skill_prompt(base_dir: &Path) -> String {
    let workspace = base_dir.join("workspace");
    let loader = FileSkillLoader::new(&workspace);
    let skills = match loader.list() {
        Ok(s) => s,
        Err(_) => return String::new(),
    };
    skills
        .iter()
        .filter(|s| !s.content.is_empty())
        .map(|s| s.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Merge skill content with an optional user-provided system prompt.
pub fn merge_prompts(skill_prompt: &str, user_prompt: &Option<String>) -> String {
    match user_prompt {
        Some(up) if !up.is_empty() => format!("{}\n\n{}", skill_prompt, up),
        _ => skill_prompt.to_string(),
    }
}

/// Generate a preamble with the current local date, time, and timezone.
///
/// Example: `"Current date and time: Saturday, March 1, 2026 at 10:30:15 AM GMT+1"`
///
/// This preamble is intentionally richer than any date metadata injected by
/// LLM providers (e.g. Anthropic's `"Current date: 2026-03-01"`). It includes:
/// - **Day-of-week** (e.g. "Saturday") — useful for scheduling context
/// - **Full time with seconds** — critical for cron scheduling precision
/// - **Timezone identifier** — essential for timezone-aware tasks
///
/// The resulting duplication with provider-side date metadata is expected and
/// harmless; this preamble is authoritative for time-aware agent operations.
/// See issue #104 for discussion.
pub fn datetime_preamble() -> String {
    let now = Local::now();
    // Format: "Saturday, March 1, 2026 at 10:30:15 AM GMT+1"
    let date_str = now.format("%A, %B %-d, %Y at %I:%M:%S %p %Z").to_string();
    format!("Current date and time: {}", date_str)
}

/// Build a complete system prompt with datetime preamble, skills, and user prompt.
///
/// Always prepends the current date/time/timezone so the agent knows
/// what "today" and "now" mean — critical for cron scheduling and
/// time-aware tasks. Combines:
/// 1. Datetime preamble (always present, see [`datetime_preamble`])
/// 2. Skill content (if any skills are loaded)
/// 3. User-provided system prompt (if any)
///
/// Note: Some LLM providers inject their own date metadata (e.g. "Current
/// date: 2026-03-01"). The quecto preamble is richer (day-of-week, full
/// time, timezone) and takes precedence for time-aware operations.
/// This duplication is intentional — see issue #104.
pub fn build_system_prompt(skill_prompt: &str, user_prompt: &Option<String>) -> String {
    let preamble = datetime_preamble();
    let merged = merge_prompts(skill_prompt, user_prompt);
    if merged.is_empty() {
        preamble
    } else {
        format!("{}\n\n{}", preamble, merged)
    }
}

/// Append the workflow state snippet to a system prompt if workflow is enabled.
pub fn append_workflow_prompt(
    system: &mut String,
    wf_config: &crate::domain::workflow::WorkflowConfig,
) {
    if !wf_config.enabled {
        return;
    }
    let state = crate::domain::workflow::WorkflowState::from_config(wf_config);
    system.push_str("\n\n");
    system.push_str(&state.system_prompt_snippet_with_config(wf_config.enforce_commit_after_step));
}

/// Register the workflow tool and guard in a tool registry if workflow is enabled.
///
/// Registers:
/// 1. The workflow tool (so the LLM can check/uncheck steps)
/// 2. A workflow guard (blocks `git commit`/`git push` at the wrong workflow stage)
pub fn register_workflow_tool(
    registry: &mut crate::infrastructure::tools::registry::ToolRegistryImpl,
    wf_config: &crate::domain::workflow::WorkflowConfig,
) {
    if !wf_config.enabled {
        return;
    }
    let state = std::sync::Arc::new(std::sync::Mutex::new(
        crate::domain::workflow::WorkflowState::from_config(wf_config),
    ));

    // Register tool
    let mut tool = crate::infrastructure::tools::workflow_tool::WorkflowTool::new(state.clone());
    tool.set_enforce_commit(wf_config.enforce_commit_after_step);
    registry.register(std::sync::Arc::new(tool));

    // Register guard (blocks git commit/push at wrong workflow stage)
    let guard = crate::infrastructure::tools::workflow_tool::WorkflowGuard::new(
        state,
        wf_config.enforce_commit_after_step,
    );
    registry.register_guard(std::sync::Arc::new(guard));
}

/// Resolve an API key for a provider from a credential snapshot.
///
/// The credential store snapshot takes priority over the config-file key.
/// Expired credentials are ignored (falls back to config key).
/// Safety margin (seconds) subtracted from OAuth `expires_in` when computing
/// `expires_at`. Compensates for clock skew and network latency so tokens are
/// refreshed before they actually expire on the server side.
pub const OAUTH_EXPIRY_MARGIN_SECS: i64 = 300;

/// Calculate `expires_at` timestamp with a consistent safety margin.
///
/// Returns `now + expires_in - OAUTH_EXPIRY_MARGIN_SECS`. Used by all
/// credential storage paths (login, import, refresh) to ensure a uniform
/// 5-minute buffer before server-side token expiration.
pub fn expires_at_with_margin(expires_in: u64) -> i64 {
    chrono::Utc::now().timestamp() + expires_in as i64 - OAUTH_EXPIRY_MARGIN_SECS
}

/// Operates on a pre-loaded snapshot to avoid redundant file I/O.
pub fn resolve_api_key(
    config_key: &str,
    creds: &HashMap<String, Credential>,
    provider: &str,
) -> String {
    if let Some(cred) = creds.get(provider) {
        if !cred.is_expired() {
            return cred.token.clone();
        }
    }
    config_key.to_string()
}

/// Resolve an API key for a provider, automatically refreshing expired OAuth tokens.
///
/// If the credential is expired and has a refresh token, attempts to refresh it
/// and update the credential store. Falls back to config key on failure.
pub fn resolve_api_key_with_refresh(
    config_key: &str,
    store: &crate::infrastructure::auth::credential_store::CredentialStore,
    provider: &str,
    rt: &tokio::runtime::Runtime,
) -> String {
    let creds = store.load_snapshot().unwrap_or_default();

    if let Some(cred) = creds.get(provider) {
        if !cred.is_expired() {
            return cred.token.clone();
        }

        // Token is expired — try to refresh if we have a refresh token
        if cred.method == crate::infrastructure::auth::credential_store::AuthMethod::OAuth {
            if let Some(ref refresh_token) = cred.refresh_token {
                if let Some(oauth_config) =
                    crate::infrastructure::auth::oauth::OAuthConfig::for_provider(provider)
                {
                    tracing::info!("refreshing expired OAuth token for {}", provider);

                    // Dispatch to the correct refresh function based on provider
                    let refresh_result = match provider {
                        "openai" => {
                            rt.block_on(crate::infrastructure::auth::oauth::refresh_openai_token(
                                &oauth_config,
                                refresh_token,
                            ))
                        }
                        _ => rt.block_on(
                            crate::infrastructure::auth::oauth::refresh_anthropic_token(
                                &oauth_config,
                                refresh_token,
                            ),
                        ),
                    };

                    if let Some(token) =
                        persist_refreshed_token(store, provider, refresh_token, refresh_result)
                    {
                        return token;
                    }
                }
            }
        }
    }

    config_key.to_string()
}

/// Resolve an API key for a provider, automatically refreshing expired OAuth tokens.
///
/// Async variant for use in the gateway (already running inside a tokio runtime).
/// If the credential is expired and has a refresh token, attempts to refresh it
/// and update the credential store. Falls back to config key on failure.
///
/// Uses the standard OAuth config for the provider. For testing with custom
/// OAuth endpoints, use [`resolve_api_key_with_refresh_async_with_oauth_config`].
pub async fn resolve_api_key_with_refresh_async(
    config_key: &str,
    store: &crate::infrastructure::auth::credential_store::CredentialStore,
    provider: &str,
) -> String {
    let oauth_config = crate::infrastructure::auth::oauth::OAuthConfig::for_provider(provider);
    match oauth_config {
        Some(ref cfg) => {
            resolve_api_key_with_refresh_async_with_oauth_config(config_key, store, provider, cfg)
                .await
        }
        None => {
            // No OAuth config for this provider — fall back to snapshot-based resolution
            let creds = store.load_snapshot().unwrap_or_default();
            resolve_api_key(config_key, &creds, provider)
        }
    }
}

/// Resolve an API key for a provider with async refresh using a custom OAuth config.
///
/// This is the testable inner function that accepts an explicit `OAuthConfig`,
/// allowing tests to point at a mock OAuth server.
pub async fn resolve_api_key_with_refresh_async_with_oauth_config(
    config_key: &str,
    store: &crate::infrastructure::auth::credential_store::CredentialStore,
    provider: &str,
    oauth_config: &crate::infrastructure::auth::oauth::OAuthConfig,
) -> String {
    let creds = store.load_snapshot().unwrap_or_default();

    if let Some(cred) = creds.get(provider) {
        if !cred.is_expired() {
            return cred.token.clone();
        }

        // Token is expired — try to refresh if we have a refresh token
        if cred.method == crate::infrastructure::auth::credential_store::AuthMethod::OAuth {
            if let Some(ref refresh_token) = cred.refresh_token {
                tracing::info!("refreshing expired OAuth token for {} (async)", provider);

                // Dispatch to the correct refresh function based on provider
                let refresh_result = match provider {
                    "openai" => {
                        crate::infrastructure::auth::oauth::refresh_openai_token(
                            oauth_config,
                            refresh_token,
                        )
                        .await
                    }
                    _ => {
                        crate::infrastructure::auth::oauth::refresh_anthropic_token(
                            oauth_config,
                            refresh_token,
                        )
                        .await
                    }
                };

                if let Some(token) =
                    persist_refreshed_token(store, provider, refresh_token, refresh_result)
                {
                    return token;
                }
            }
        }
    }

    config_key.to_string()
}

/// Process an OAuth token refresh result: build and persist the new credential.
///
/// Returns `Some(access_token)` on success, `None` on failure (logged as warning).
/// Shared by both sync and async refresh paths to avoid credential-building duplication.
///
/// `previous_refresh_token` is preserved when the server response omits
/// `refresh_token` (valid per RFC 6749 §5.1 — the field is OPTIONAL).
fn persist_refreshed_token(
    store: &crate::infrastructure::auth::credential_store::CredentialStore,
    provider: &str,
    previous_refresh_token: &str,
    refresh_result: Result<
        crate::infrastructure::auth::oauth::OAuthTokenResponse,
        crate::domain::error::DomainError,
    >,
) -> Option<String> {
    match refresh_result {
        Ok(token_resp) => {
            let expires_at = expires_at_with_margin(token_resp.expires_in);
            let account_id = if provider == "openai" {
                crate::infrastructure::auth::oauth::extract_openai_account_id(
                    &token_resp.access_token,
                )
            } else {
                None
            };
            let effective_refresh = token_resp
                .refresh_token
                .unwrap_or_else(|| previous_refresh_token.to_string());
            let new_cred = Credential {
                provider: provider.to_string(),
                token: token_resp.access_token.clone(),
                method: crate::infrastructure::auth::credential_store::AuthMethod::OAuth,
                expires_at: Some(expires_at),
                refresh_token: Some(effective_refresh),
                account_id,
            };
            if let Err(e) = store.store(new_cred) {
                tracing::warn!("failed to persist refreshed token for {}: {}", provider, e);
            }
            Some(token_resp.access_token)
        }
        Err(e) => {
            tracing::warn!("failed to refresh OAuth token for {}: {}", provider, e);
            None
        }
    }
}

/// Check which providers have expired credentials and need re-authentication.
///
/// Operates on a pre-loaded snapshot to avoid redundant file I/O.
/// Resolve the effective workspace directory for an agent or REPL invocation.
///
/// When `no_sandbox` is `true`, the agent should operate from the **process's
/// current working directory** rather than the configured workspace path. This
/// lets users run `quecto --no-sandbox` from any directory and have the agent
/// see that directory as its root, matching how every other CLI tool behaves.
///
/// When sandbox is enabled (the default), the configured workspace path is used.
///
/// # Arguments
///
/// * `config_workspace` — the resolved workspace path from config (already `~`-expanded)
/// * `no_sandbox` — whether the `--no-sandbox` flag was passed
pub fn resolve_agent_workspace(config_workspace: &str, no_sandbox: bool) -> std::path::PathBuf {
    if no_sandbox {
        match std::env::current_dir() {
            Ok(cwd) => cwd,
            Err(e) => {
                // CWD is unavailable (e.g. deleted directory). Fall back to the
                // config workspace and emit a warning so the user is not silently
                // misled about which directory the agent is operating from.
                tracing::warn!(
                    error = %e,
                    fallback = config_workspace,
                    "--no-sandbox: current_dir() failed, falling back to config workspace"
                );
                std::path::PathBuf::from(config_workspace)
            }
        }
    } else {
        std::path::PathBuf::from(config_workspace)
    }
}

/// Build a [`RefreshFn`] for use with [`RefreshableProvider`].
///
/// The returned function reads the stored refresh token, calls the appropriate
/// OAuth refresh endpoint, persists the new credential, and returns the new
/// access token.
pub fn make_oauth_refresh_fn() -> crate::infrastructure::providers::refreshable::RefreshFn {
    use std::sync::Arc;
    Arc::new(|store, provider_name| {
        let provider_name = provider_name.to_string();
        let store = store.clone();
        Box::pin(async move {
            let creds = store.load_snapshot().unwrap_or_default();
            let cred = creds.get(&provider_name).ok_or_else(|| {
                crate::domain::error::DomainError::Provider(format!(
                    "no credential found for {}",
                    provider_name
                ))
            })?;
            let refresh_token = cred.refresh_token.as_ref().ok_or_else(|| {
                crate::domain::error::DomainError::Provider(format!(
                    "no refresh token for {}",
                    provider_name
                ))
            })?;
            let oauth_config =
                crate::infrastructure::auth::oauth::OAuthConfig::for_provider(&provider_name)
                    .ok_or_else(|| {
                        crate::domain::error::DomainError::Provider(format!(
                            "no OAuth config for {}",
                            provider_name
                        ))
                    })?;

            let refresh_result = match provider_name.as_str() {
                "openai" => {
                    crate::infrastructure::auth::oauth::refresh_openai_token(
                        &oauth_config,
                        refresh_token,
                    )
                    .await
                }
                _ => {
                    crate::infrastructure::auth::oauth::refresh_anthropic_token(
                        &oauth_config,
                        refresh_token,
                    )
                    .await
                }
            };

            persist_refreshed_token(&store, &provider_name, refresh_token, refresh_result)
                .ok_or_else(|| {
                    crate::domain::error::DomainError::Provider(format!(
                        "failed to refresh token for {}",
                        provider_name
                    ))
                })
        })
    })
}

/// Build a [`ProviderFactory`] that re-creates a provider with a new API key.
///
/// The factory knows the provider name and API base URL, and creates the
/// correct provider type (Codex for OpenAI OAuth, standard otherwise).
pub fn make_provider_factory(
    provider_name: &str,
    api_base: Option<String>,
    http_client: reqwest::Client,
) -> crate::infrastructure::providers::refreshable::ProviderFactory {
    use crate::infrastructure::providers;
    use std::sync::Arc;

    let name = provider_name.to_string();
    let base = api_base;
    Arc::new(
        move |new_token: &str| -> Arc<dyn crate::domain::provider::LlmProvider> {
            if name == "openai" {
                let account_id =
                    crate::infrastructure::auth::oauth::extract_openai_account_id(new_token);
                if let Some(acct) = account_id {
                    return providers::create_codex_provider_with_client(
                        new_token.to_string(),
                        acct,
                        http_client.clone(),
                    );
                }
            }
            match providers::create_provider_with_client(
                &name,
                new_token.to_string(),
                base.clone(),
                http_client.clone(),
            ) {
                Ok(p) => p,
                Err(e) => {
                    tracing::error!(
                        provider = name.as_str(),
                        error = %e,
                        "failed to rebuild provider after token refresh"
                    );
                    // Return a provider that will fail — better than panicking
                    providers::create_provider_with_client(
                        &name,
                        new_token.to_string(),
                        None,
                        http_client.clone(),
                    )
                    .unwrap_or_else(|_| {
                        Arc::new(
                            crate::infrastructure::providers::openai::OpenAiProvider::new(
                                new_token.to_string(),
                                None,
                            ),
                        )
                    })
                }
            }
        },
    )
}

pub fn check_provider_readiness(creds: &HashMap<String, Credential>) -> Vec<String> {
    creds
        .values()
        .filter(|c| c.is_expired())
        .map(|c| c.provider.clone())
        .collect()
}

#[cfg(test)]
#[path = "shared_tests.rs"]
mod tests;
