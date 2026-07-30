//! Shared utility functions used by CLI and REPL modules.

use crate::infrastructure::auth::credential_store::Credential;
use std::collections::HashMap;

/// Generate a fresh, collision-resistant user-chat session key.
///
/// The domain owns the key *shape* ([`crate::domain::session::user_chat_key`]);
/// this interface helper owns the impure inputs — the wall clock plus a
/// uniqueness token combining the process id with a per-process counter — so two
/// launches started in the same second (or two chats within one process) never
/// collide on a key.
pub fn generate_chat_key() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    // PID disambiguates separate launches; the counter disambiguates within one.
    let uniq = ((std::process::id() as u64) << 24) ^ seq;
    crate::domain::session::user_chat_key(secs, uniq)
}

/// Scrub the ephemeral (empty-key) spill file at run end; no-op for named
/// sessions. Shared by every ephemeral interface exit path (one-shot CLI,
/// UDS server, REPL). See `FileContextSpillStore::scrub_session_spill_sync`:
/// ephemeral runs persist spilled content only so the run's own recall()
/// stubs resolve, and it must never survive the run (PR #1048 security
/// review).
pub fn scrub_ephemeral_spill(base_dir: &std::path::Path, ephemeral: bool) {
    if ephemeral {
        crate::infrastructure::persistence::context_spill::FileContextSpillStore::
            scrub_session_spill_sync(base_dir, "");
    }
}

/// Merge an optional user-provided system prompt.
pub fn merge_prompts(user_prompt: &Option<String>) -> String {
    match user_prompt {
        Some(up) if !up.is_empty() => up.to_string(),
        _ => String::new(),
    }
}

/// Stable parent-agent identity injected into top-level agent system prompts.
///
/// Short on purpose (~one paragraph). Full coordination rules live in the
/// `docs` operating manual (`quick-start`), not here. Spawned children omit this
/// (#1319) so they do not re-delegate under parent guidance.
pub fn agent_role_preamble() -> &'static str {
    "You are the Parent Agent operating inside Quecto, an agentic coding harness that can spawn full-featured replicas of itself. Use subagents to isolate substantial working context and run independent work in the background while you, the parent, remain available to the user."
}

/// Compact Quecto capability signpost injected into top-level agent system prompts.
///
/// This is intentionally a retrieval policy, not full documentation. It tells
/// parent agents where to look only when Quecto-specific operational knowledge is
/// needed, keeping normal prompt/context usage small. Spawned children omit this
/// parent-oriented policy (#1319).
pub fn agent_docs_retrieval_policy() -> &'static str {
    "The `docs` tool is Quecto's operating manual - your definitive source for how Quecto works. For parent coordination, delegation, workflows, or Quecto-specific behavior, start with `docs {\"name\": \"quick-start\"}`; open other manual pages only when that knowledge is needed. Keep context lean."
}

/// Build a complete system prompt for a top-level or spawned agent (#1319).
///
/// Top-level (`spawned = false`) combines:
/// 1. Parent-agent role preamble ([`agent_role_preamble`])
/// 2. Quecto operating-manual retrieval policy ([`agent_docs_retrieval_policy`])
/// 3. User-provided system prompt (if any)
///
/// Spawned children (`spawned = true`) omit parent identity and parent
/// coordination/docs policy, retaining only the optional explicit system
/// prompt (e.g. `spawn.system`). Tool schemas, the initial task, and workflow
/// guidance are supplied separately by the runtime.
///
/// Providers may still inject their own date metadata; Quecto no longer
/// prepends a local datetime preamble.
pub fn build_system_prompt(user_prompt: &Option<String>, spawned: bool) -> String {
    let merged = merge_prompts(user_prompt);
    if spawned {
        return merged;
    }
    let role = agent_role_preamble();
    let docs_policy = agent_docs_retrieval_policy();
    if merged.is_empty() {
        format!("{}\n\n{}", role, docs_policy)
    } else {
        format!("{}\n\n{}\n\n{}", role, docs_policy, merged)
    }
}

/// Append extension system prompt snippets in a clearly delimited section.
///
/// Wrapping prevents extension snippets from being misinterpreted as core
/// system instructions by the LLM.
pub fn append_extension_prompt(system: &mut String, snippets: &str) {
    if !snippets.is_empty() {
        system.push_str("\n\n## Extensions\n");
        system.push_str(snippets);
        system.push_str("\n## End Extensions");
    }
}

/// Shared workflow engine handle returned by [`register_workflow_tool`].
///
/// Workflow state is deliberately NEVER rendered into the system prompt
/// (#1113): the prompt stays byte-identical for the whole session so the
/// provider-side cached prefix survives every workflow step. Dynamic state
/// reaches the model through workflow tool results and idle-boundary nudges.
pub type WorkflowStateHandle =
    std::sync::Arc<std::sync::Mutex<crate::domain::workflow::WorkflowEngine>>;

/// Register the workflow tool and optional guard in a tool registry.
///
/// Tool construction goes through the bundled native provider seam (#1276 Phase 3)
/// while engine-handle sharing and optional guard registration stay identical.
pub fn register_workflow_tool(
    registry: &mut crate::infrastructure::tools::registry::ToolRegistryImpl,
    wf_config: crate::domain::workflow::WorkflowConfig,
    guards_enabled: bool,
    event_emitter: Option<crate::infrastructure::tools::workflow_tool::WorkflowEventEmitter>,
) -> Result<WorkflowStateHandle, crate::domain::workflow::WorkflowError> {
    use crate::infrastructure::extensions::native::{
        WorkflowToolDeps, build_workflow_tool_extension, register_bundled_native_tools,
    };

    let engine: WorkflowStateHandle = std::sync::Arc::new(std::sync::Mutex::new(
        crate::domain::workflow::WorkflowEngine::new(wf_config, guards_enabled)?,
    ));

    register_bundled_native_tools(
        registry,
        vec![build_workflow_tool_extension(WorkflowToolDeps {
            engine: engine.clone(),
            event_emitter,
        })],
    );

    if guards_enabled {
        let guard = crate::infrastructure::tools::workflow_tool::WorkflowGuard::new(engine.clone());
        registry.register_guard(std::sync::Arc::new(guard));
    }

    Ok(engine)
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
    crate::infrastructure::time::unix_timestamp_secs() + expires_in as i64
        - OAUTH_EXPIRY_MARGIN_SECS
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
/// Sync wrapper around [`resolve_api_key_with_refresh_async`] for callers that
/// hold a `tokio::runtime::Runtime` but are not inside an async context (e.g.
/// the CLI agent entrypoint). Eliminates duplicated refresh/persist logic (#308).
///
/// # Panics
///
/// Panics if called from within an active tokio runtime context (i.e. inside a
/// `.await` chain or a `tokio::spawn` task). Use [`resolve_api_key_with_refresh_async`]
/// instead in those contexts.
pub fn resolve_api_key_with_refresh(
    config_key: &str,
    store: &crate::infrastructure::auth::credential_store::CredentialStore,
    provider: &str,
    rt: &tokio::runtime::Runtime,
) -> String {
    debug_assert!(
        tokio::runtime::Handle::try_current().is_err(),
        "resolve_api_key_with_refresh called inside an active tokio runtime — use resolve_api_key_with_refresh_async instead"
    );
    rt.block_on(resolve_api_key_with_refresh_async(
        config_key, store, provider,
    ))
}

/// Resolve an API key for a provider, automatically refreshing expired OAuth tokens.
///
/// Async variant for callers already running inside a tokio runtime.
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
                    "xai" => {
                        crate::infrastructure::auth::oauth::refresh_xai_token(
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
    let workspace = if no_sandbox {
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
    };
    // Zero-config: ensure the workspace exists (onboarding used to create it).
    // Best-effort — a failure surfaces later as a clear filesystem error.
    if let Err(e) = std::fs::create_dir_all(&workspace) {
        tracing::warn!(
            error = %e,
            workspace = %workspace.display(),
            "failed to create workspace directory"
        );
    }
    workspace
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
                "xai" => {
                    crate::infrastructure::auth::oauth::refresh_xai_token(
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

            let token =
                persist_refreshed_token(&store, &provider_name, refresh_token, refresh_result)
                    .ok_or_else(|| {
                        crate::domain::error::DomainError::Provider(format!(
                            "failed to refresh token for {}",
                            provider_name
                        ))
                    })?;

            // Best-effort: push the refreshed credentials back to the runtime
            // manager so the shared Secret (and therefore newly spawned pods)
            // start from a fresh, non-expired token. Failure here must not fail
            // the refresh — the in-process token is already valid.
            sync_credentials_to_manager(store.path()).await;

            Ok(token)
        })
    })
}

/// Push the local `credentials.json` to the runtime manager's credential sync
/// endpoint, if configured via `QUECTO_CREDENTIAL_SYNC_URL`.
///
/// Best-effort and non-fatal: any failure is logged and swallowed. When the env
/// var is unset (e.g. local CLI use, no cluster manager), this is a no-op.
async fn sync_credentials_to_manager(credentials_path: &std::path::Path) {
    let Ok(url) = std::env::var("QUECTO_CREDENTIAL_SYNC_URL") else {
        return;
    };
    if url.trim().is_empty() {
        return;
    }

    let credentials_json = match tokio::fs::read_to_string(credentials_path).await {
        Ok(contents) => contents,
        Err(e) => {
            tracing::warn!(error = %e, "credential sync: failed to read credentials file");
            return;
        }
    };

    let mut request = reqwest::Client::new()
        .put(&url)
        .json(&serde_json::json!({ "credentials_json": credentials_json }));

    if let Ok(token) = std::env::var("QUECTO_CREDENTIAL_SYNC_TOKEN") {
        let token = token.trim();
        if !token.is_empty() {
            request = request.bearer_auth(token);
        }
    }

    match request.send().await {
        Ok(resp) if resp.status().is_success() => {
            tracing::info!("credential sync: pushed refreshed credentials to runtime manager");
        }
        Ok(resp) => {
            tracing::warn!(status = %resp.status(), "credential sync: manager rejected update");
        }
        Err(e) => {
            tracing::warn!(error = %e, "credential sync: request to manager failed");
        }
    }
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
                    // `base` was already validated when the original provider
                    // was constructed; an invalid base cannot appear here, but
                    // degrade to the hardwired ChatGPT backend rather than
                    // panic inside the refresh path.
                    match providers::create_codex_provider_with_client(
                        new_token.to_string(),
                        acct.clone(),
                        base.clone(),
                        http_client.clone(),
                    ) {
                        Ok(p) => return p,
                        Err(e) => {
                            tracing::error!(
                                error = %e,
                                "invalid openai api_base at token refresh; using default backend"
                            );
                            return providers::create_codex_provider_with_client(
                                new_token.to_string(),
                                acct,
                                None,
                                http_client.clone(),
                            )
                            .expect("default Codex backend is always valid");
                        }
                    }
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

/// Build a shared HTTP client with sensible timeouts.
///
/// Used by providers and native extensions to share a single connection pool
/// and TLS context. Important on memory-constrained targets (RQuecto, containers).
pub fn build_http_client() -> reqwest::Client {
    // No overall timeout — SSE streams legitimately run for minutes during
    // long LLM generations. The connect_timeout gates the initial handshake;
    // per-request timeouts are set at the call site when needed (e.g.
    // web_fetch uses its own 10s timeout).
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default()
}

/// Build and register native (config-gated) extensions.
///
/// Returns an `ExtensionRegistry` containing native extensions.
/// Native extensions are evaluated once at agent construction and are not
/// affected by `reload_extensions`. Changes to config require an agent restart.
pub fn build_and_register_native_extensions(
    config: &crate::infrastructure::config::Config,
    http_client: &reqwest::Client,
) -> crate::infrastructure::extensions::registry::ExtensionRegistry {
    let mut ext_registry = crate::infrastructure::extensions::registry::ExtensionRegistry::new();
    for ext in crate::infrastructure::extensions::native::build_native_extensions(
        &config.tools.web,
        http_client,
    ) {
        ext_registry.register(ext);
    }
    ext_registry
}

/// Register extension tools, rejecting any that shadow core tools.
pub fn register_extension_tools(
    registry: &mut crate::infrastructure::tools::registry::ToolRegistryImpl,
    ext_registry: &crate::infrastructure::extensions::registry::ExtensionRegistry,
) {
    for tool in ext_registry.all_tools() {
        // `register_extension` tracks the tool as an extension and rejects
        // shadows of core tools automatically.
        registry.register_extension(tool);
    }
}

/// Resolve the XDG runtime directory or fall back to temp.
///
/// Returns `$XDG_RUNTIME_DIR` if it exists, is a directory, and is writable.
/// Otherwise returns `std::env::temp_dir()`.
pub fn xdg_runtime_dir_or_temp() -> std::path::PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_RUNTIME_DIR") {
        let path = std::path::PathBuf::from(xdg);
        if path.is_dir() {
            let probe = path.join(".quecto-probe");
            if std::fs::File::create(&probe).is_ok() {
                let _ = std::fs::remove_file(&probe);
                return path;
            }
        }
    }
    std::env::temp_dir()
}

#[cfg(test)]
#[path = "shared_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "shared_cov_tests.rs"]
mod cov_tests;
