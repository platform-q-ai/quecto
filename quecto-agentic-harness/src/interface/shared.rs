//! Shared utility functions used by CLI and REPL modules.

use crate::infrastructure::auth::credential_store::Credential;
use crate::infrastructure::auth::token_refresh::persist_refreshed_token;
use std::collections::HashMap;

/// Merge an optional user-provided system prompt.
pub fn merge_prompts(user_prompt: &Option<String>) -> String {
    match user_prompt {
        Some(up) if !up.is_empty() => up.to_string(),
        _ => String::new(),
    }
}

/// Parent identity and coordination policy; never injected into spawned children.
pub fn agent_role_preamble() -> &'static str {
    "You are the Parent Agent operating inside Quecto, an agentic coding harness. Own user communication, scope, orchestration, approvals, and synthesis; keep substantial working context in scoped swarms rather than in the parent while you, the parent, remain available to the user."
}

/// Parent-only routing policy, kept out of the shared manual.
fn parent_coordination_policy() -> &'static str {
    include_str!("../../../PARENT_PLAYBOOK.md").trim_ascii_end()
}

/// Child ownership boundary permits useful decomposition without coordinator chains.
fn child_role_preamble() -> &'static str {
    "You are a subagent responsible for the assigned task. Solve it directly by default. You may delegate a bounded, independently useful subtask when doing so materially improves the result. Do not delegate your entire assignment, create another coordinator for the same task, or spawn agents merely to reduce your own context. Remain responsible for integrating and verifying delegated results."
}

fn core_system_prompt_with_playbook(spawned: bool, playbook: &str) -> String {
    let role = if spawned {
        child_role_preamble()
    } else {
        agent_role_preamble()
    };
    let mut sections = vec![role];
    if !spawned {
        sections.push(playbook);
    }
    sections.join("\n\n")
}

fn core_system_prompt(spawned: bool) -> String {
    core_system_prompt_with_playbook(spawned, parent_coordination_policy())
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
    build_agent_system_prompt_with_playbook(
        agents_instructions,
        user_prompt,
        spawned,
        extension_snippets,
        parent_coordination_policy(),
    )
}

/// Compose startup sources with a parent-only playbook selected at initialization.
pub fn build_agent_system_prompt_with_playbook(
    agents_instructions: Option<&str>,
    user_prompt: Option<&str>,
    spawned: bool,
    extension_snippets: &str,
    playbook: &str,
) -> String {
    let mut prompt = core_system_prompt_with_playbook(spawned, playbook);
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
        crate::composition::find::build_find_tool(
            std::sync::Arc::new(workspace.clone()),
            std::sync::Arc::new(sandbox.clone()),
        ),
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
    web_fetch_tool: Option<std::sync::Arc<dyn crate::application::tools::ports::Tool>>,
) -> crate::infrastructure::extensions::registry::ExtensionRegistry {
    let mut ext_registry = crate::infrastructure::extensions::registry::ExtensionRegistry::new();
    for ext in crate::infrastructure::extensions::native::build_native_extensions(
        &config.tools.web,
        http_client,
        web_fetch_tool,
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

#[cfg(test)]
#[path = "shared_parent_policy_tests.rs"]
mod parent_policy_tests;
