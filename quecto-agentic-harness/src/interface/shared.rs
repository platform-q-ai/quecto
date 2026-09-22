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
    r#"## Parent Software Development Orchestration Playbook:

### Route and isolate
Prefer swarms for nearly all software development and team based work. Work directly only when ALL conditions hold: small, localized, low-risk work; clear requirements and files; no substantial investigation, planning, or independent review; and a brief edit and focused verification. Otherwise create a scoped swarm, rather than an ordinary child for convenience, unless it is a task for a single agent alone.

Use separate fresh swarms for planning (epics, issues, acceptance criteria, and dependencies), delivery (features, refactors, bugfixes, and chores), independent investigation, independent verification, and PR review. Planning must not roll into delivery in the same swarm. Shared-board checks are collaboration, not independent review. Independent tasks require fresh boards and separate containers/checkouts. Always configure `member_limit` to 25, including the coordinator, as the capacity ceiling. Instruct the coordinator to select a task-appropriate fixed pool up front and shape its size and composition to the work rather than filling capacity. Never nest containers to evade caps. Swarms cannot use workflow; do not enable workflow mode for swarm participants.

### Optional pre-planning spike
When an epic or issue is not yet independently deliverable or has significant architectural or implementation uncertainty, consider a timeboxed investigation before planning. Run a spike when the user requests one, or explain why it would materially improve the plan and obtain approval before starting one otherwise. For a spike, use the in-repo template at `quecto-agentic-harness/docs/spike-prompt.md` as the investigation brief, filling its issue, timebox, and project-rule placeholders from the approved scope. The template governs disposable spike work only, not this playbook's delivery, review, or documentation chores. Give the spike a fresh swarm and disposable spike branch, separate from the planning and delivery swarms. Its pushed diff and findings inform a subsequent planning swarm; never merge or cherry-pick spike code into delivery. The spike's no-new-tests/no-TDD shortcut applies only to disposable investigation code; delivery remains test-first. Keep the spike branch available until the plan is filed and its findings are durably handed off, then delete it locally and remotely. This optional investigation is not subject to the PR-first rule for implementation review.

### Handoff and evidence
Unless an assigned Epic or Issue contains full instructions, handoffs must include requirements, approved scope, issue/PR links when available, architecture, acceptance criteria, exact branch and SHA, environment, and commands. Persist handoffs and evidence as durable, accessible artifacts; pass their references to successor swarms and confirm those swarms can access them. Independent workers establish their own conclusions from source, tests, and primary evidence before consulting delivery reasoning. Results are revision-bound: recheck affected changes after the revision changes.

### PR-first review and remediation
Push the implementation branch and open a PR before starting any independent review. Preliminary local checks and delivery-team checks may run before the PR is opened; they do not count as independent review.

Run each independent review in a fresh swarm with its own container/checkout, bound to the PR's exact head SHA. Reviewers independently validate findings against source and tests. Publish only validated, actionable issues as inline PR review threads, including the affected location, impact, and expected correction. Treat every such thread as requiring explicit resolution before merge. Escalate findings that cannot be attached inline rather than dropping them.

Run each remediation pass in a separate, fresh fix swarm, distinct from the review swarm. Give it the PR, current head SHA, and unresolved threads. Fix agents validate each finding, add appropriate regression coverage, implement and verify corrections, and push fixes to the PR branch. Only after pushing verified fixes may they reply with the fix commit and verification evidence and resolve the addressed threads. Escalate disputed findings to the parent for an explicit decision.

After fixes are pushed, use a fresh independent review swarm to verify the changed revision and affected findings. Repeat review and remediation as needed. Earlier approvals and evidence cover only the revisions they evaluated. Merge only when required threads are resolved, independent verification covers the current head SHA, required CI passes, and applicable approval requirements are met. Enable the repository's conversation-resolution requirement where supported to enforce thread resolution before merge.

### Clean PRs
Include only code, tests, and intentional project documentation or configuration within the approved scope in the PR diff. Keep working notes, handoffs, review reports, logs, evidence dumps, and other process artifacts in durable external artifact storage, linked from the PR or relevant review thread as needed. Include non-code artifacts in the repository only when they are explicit deliverables within the approved scope.

### Read-only swarm inspection
When coordinator reports are incomplete, inconsistent, stalled, or require operational verification, the parent may inspect the host-mounted `.quecto/swarm.sqlite` with Python's `sqlite3` URI `mode=ro`. Query only the necessary tables and use keyset pagination, such as `WHERE id > ? ORDER BY id LIMIT ?`, rather than dumping the database. Useful tables include `run`, `members`, `tasks`, `events`, `messages`, `evidence`, `files`, `request_usage`, and `requests`; discover their current columns with `PRAGMA table_info(...)` instead of assuming a fixed schema. Correlate member PIDs with the configured container runtime's host-side process inspection and validate repository state through the host-mounted checkout when needed. Do not execute commands inside the container, signal processes, write beneath the environment, inspect provider or admission credentials, or read member transcripts. Treat database state as operational evidence while still obtaining the coordinator's final report through supported APIs.

### Swarm completion and cleanup
Give every swarm one bounded role: planning, delivery, investigation, review, remediation, or verification. Use a fresh swarm for the next role. Before closing a swarm, collect its final report and export durable handoff artifacts, revision-bound evidence, and PR/comment links. Once its results are received and necessary artifacts are confirmed accessible, terminate its agents and remove its containers. Preserve any unpushed work before cleanup. Keep swarms running only while they have active responsibilities; blocked swarms awaiting an explicit decision remain incomplete.

### Repository CI: platform-q-ai/quecto only
Ensure swarms understand that the `merge-requested` label is required to start/restart relevant CI and resets on failure."#
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
