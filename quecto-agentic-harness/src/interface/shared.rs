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
/// UDS server). See `FileContextSpillStore::scrub_session_spill_sync`:
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

/// Parent identity and coordination policy; never injected into spawned children.
pub fn agent_role_preamble() -> &'static str {
    "You are the Parent Agent operating inside Quecto, an agentic coding harness. Own user communication, scope, orchestration, approvals, and synthesis; keep substantial working context in scoped swarms rather than in the parent."
}

/// Parent-only routing policy, kept out of the shared manual.
fn parent_coordination_policy() -> &'static str {
    r#"## Parent orchestration playbook

### Route and isolate
Prefer swarms for nearly all software development. Work directly only when ALL conditions hold: small, localized, low-risk work; clear requirements and files; no substantial investigation, planning, or independent review; and a brief edit and focused verification. Otherwise create a scoped swarm, rather than an ordinary child for convenience.

Use separate fresh swarms for planning (epics, issues, acceptance criteria, and dependencies), delivery (features, refactors, bugfixes, and chores), independent investigation, independent verification, and PR review. Planning must not roll into delivery in the same swarm. Shared-board checks are collaboration, not independent review. Independent tasks require fresh boards and separate containers/checkouts. Use a fixed pool within its cap; never nest containers to evade caps. Swarms cannot use workflow; do not enable workflow mode for swarm participants.

### Handoff and evidence
Handoffs must include requirements, approved scope, issue/PR links when available, architecture, acceptance criteria, exact branch and SHA, environment, and commands. Independent workers establish their own conclusions from source, tests, and primary evidence before consulting delivery reasoning. Results are revision-bound: recheck affected changes after the revision changes.

Read delegated reports and critical evidence, reconcile disagreements, and synthesize for the user instead of duplicating delegated investigations. Before replacing work, stop or settle the previous run while preserving artifacts. When blocked, ask a precise question and retain resumable work. Missing evidence is a blocker: never infer success from missing access, failed commands, or an empty board.

### Repository CI: platform-q-ai/quecto only
The `merge-requested` label is required to start/restart relevant CI and resets on failure. Inspect labels, PR head, and workflows. Apply the label when ready only if user authorization and repository process permit: it may auto-merge. Confirm the actual run targets the intended SHA. After failure, diagnose, fix, push, then inspect/reapply the label as needed; push alone is not proof of CI restart. If the label is present but no run starts, inspect triggers. Avoid blind label toggles or retry loops. Treat this as a repository-scoped rule, not a universal CI convention.

### Completion and learning
Completion requires all applicable gates: approved scope and acceptance criteria met; a committed accessible branch; tests/CI at the current SHA; separate swarm review and required independent verification; findings resolved or user-accepted; and closing references and state checked. Report implemented, PR opened, CI passing, reviewed, merged, and issue closed as distinct states, claiming only those supported by evidence and authorization.

From corrections, propose repository-scoped durable rules recording failure, prevention, applicability, and verification. Persist approved lessons in the prompt or designated playbook rather than conversation alone. Keep secrets out of lessons and artifacts."#
}

/// Child ownership boundary permits useful decomposition without coordinator chains.
fn child_role_preamble() -> &'static str {
    "You are a subagent responsible for the assigned task. Solve it directly by default. You may delegate a bounded, independently useful subtask when doing so materially improves the result. Do not delegate your entire assignment, create another coordinator for the same task, or spawn agents merely to reduce your own context. Remain responsible for integrating and verifying delegated results."
}

fn core_system_prompt(spawned: bool) -> String {
    let role = if spawned {
        child_role_preamble()
    } else {
        agent_role_preamble()
    };
    let mut sections = vec![role];
    if !spawned {
        sections.push(parent_coordination_policy());
    }
    sections.join("\n\n")
}

fn append_prompt_section(prompt: &mut String, heading: &str, content: &str) {
    if content.is_empty() {
        return;
    }
    let delimiter = format!("<{heading}>");
    prompt.push_str("\n\n");
    prompt.push_str(&delimiter);
    prompt.push_str("\nContent length: ");
    prompt.push_str(&content.len().to_string());
    prompt.push_str(" bytes\n\n");
    prompt.push_str(content);
    prompt.push_str("\n\n</");
    prompt.push_str(heading);
    prompt.push('>');
}

/// Build the full startup system prompt in stable precedence order.
///
/// Core policy comes first, followed by initialization-directory `AGENTS.md`,
/// the explicit CLI system prompt, and extension snippets. Each optional source
/// is wrapped so adjacent instructions cannot be accidentally concatenated.
pub fn build_agent_system_prompt(
    agents_instructions: Option<&str>,
    user_prompt: Option<&str>,
    spawned: bool,
    extension_snippets: &str,
) -> String {
    let mut prompt = core_system_prompt(spawned);
    prompt.push_str("\n\n## End Core Instructions");
    if let Some(instructions) = agents_instructions {
        append_prompt_section(&mut prompt, "agents-md-instructions", instructions);
    }
    if let Some(user_prompt) = user_prompt {
        append_prompt_section(&mut prompt, "user-system-prompt", user_prompt);
    }
    append_prompt_section(&mut prompt, "extensions", extension_snippets);
    prompt
}

/// Build role-specific instructions and optional custom text.
/// Spawned children receive an ownership boundary, not parent routing policy.
/// Tool schemas, the initial task, and workflow guidance are supplied separately.
pub fn build_system_prompt(user_prompt: &Option<String>, spawned: bool) -> String {
    let merged = merge_prompts(user_prompt);
    let mut prompt = core_system_prompt(spawned);
    if !merged.is_empty() {
        prompt.push_str("\n\n");
        prompt.push_str(&merged);
    }
    prompt
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

#[path = "shared_workflow.rs"]
mod shared_workflow;
#[cfg(any(test, feature = "test-support"))]
pub use shared_workflow::register_workflow_tool;
pub use shared_workflow::register_workflow_tool_with_participation;

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
            // Rotation-aware persist: if another agent process refreshed
            // concurrently (its rotated refresh token is already on disk),
            // keep its credential instead of overwriting it (#1460 review).
            match store.store_refreshed(new_cred, previous_refresh_token) {
                Ok(authoritative) => Some(authoritative.token),
                Err(e) => {
                    tracing::warn!("failed to persist refreshed token for {}: {}", provider, e);
                    Some(token_resp.access_token)
                }
            }
        }
        Err(e) => {
            tracing::warn!("failed to refresh OAuth token for {}: {}", provider, e);
            None
        }
    }
}

/// Resolve the effective workspace directory for an agent or REPL invocation.
///
/// Quecto now runs in the process's current working directory by default. The
/// legacy configured workspace is accepted as a fallback only when the current
/// directory cannot be read.
pub fn resolve_agent_workspace(config_workspace: &str) -> std::path::PathBuf {
    let workspace = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(e) => {
            tracing::warn!(
                error = %e,
                fallback = config_workspace,
                "current_dir() failed, falling back to config workspace"
            );
            std::path::PathBuf::from(config_workspace)
        }
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
    crate::infrastructure::providers::default_client_builder()
        .build()
        .unwrap_or_default()
}

/// Build the shared official-tool catalogue/registry root used by CLI, REPL,
/// and UDS-backed agent sessions.
///
/// Entrypoints pass their intentional defaults (for example docs content policy
/// and exec capture limits) as policy inputs here instead of constructing a
/// separate production tool set. Native lifecycle remains
/// in-process/bundled; UDS lifecycle continues to register dynamic proxy tools
/// later through the runtime-loadable path.
pub fn build_official_tool_registry(
    workspace: std::path::PathBuf,
    sandbox: crate::infrastructure::security::sandbox::Sandbox,
    exec_options: crate::infrastructure::tools::bash::ExecOptions,
) -> crate::infrastructure::tools::registry::ToolRegistryImpl {
    crate::infrastructure::extensions::native::build_official_tool_registry_with_context(
        workspace,
        sandbox,
        exec_options,
        crate::interface::tool_runtime::swarm_context(),
    )
}

/// Build and register native (config-gated) extensions.
///
/// Returns an `ExtensionRegistry` containing native extensions.
/// Native extensions are evaluated once at agent construction and are not
/// affected by ``. Changes to config require an agent restart.
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

/// Register config-gated bundled native provider tools into the official tool surface.
///
/// This compatibility helper is currently used for web search/fetch. These tools
/// are native extensions in the #1276 sense: compiled into Quecto, directly
/// invoked in-process, and governed by catalogue/policy enablement. They are not
/// UDS/runtime-loadable tools, so they must use the bundled-native registration
/// path rather than the historical `register_extension` lifecycle API.
pub fn register_bundled_native_extension_tools(
    registry: &mut crate::infrastructure::tools::registry::ToolRegistryImpl,
    ext_registry: &crate::infrastructure::extensions::registry::ExtensionRegistry,
) {
    for extension in ext_registry.extensions() {
        let provider_id = extension.name().to_string();
        for tool in extension.tools() {
            registry.register_with_metadata(
                tool,
                crate::infrastructure::tools::registry::ToolRegistration::official_native()
                    .with_provider_id(provider_id.clone()),
            );
        }
    }
}

pub(crate) use crate::interface::tool_runtime::{
    ToolEntrypoint, ToolRuntimeBuildArgs, ToolRuntimeWorkflowPolicy, build_tool_runtime,
};

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
#[path = "native_provider_catalogue_tests.rs"]
mod native_provider_catalogue_tests;

#[cfg(test)]
#[path = "shared_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "shared_cov_tests.rs"]
mod cov_tests;
