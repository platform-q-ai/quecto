use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use super::CliContext;
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::domain::agent::AgentLoop;
use crate::domain::message::Message;
use crate::domain::provider::LlmProvider;
use crate::domain::session::{Session, SessionStore};
use crate::infrastructure::auth::credential_store::CredentialStore;
use crate::infrastructure::config::Config;
use crate::infrastructure::persistence::context_spill::FileContextSpillStore;
use crate::infrastructure::persistence::session_store::FileSessionStore;
use crate::infrastructure::providers;
use crate::infrastructure::providers::fallback::FallbackProvider;
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::recall::RecallTool;
use crate::infrastructure::tools::registry::ToolRegistryImpl;
use crate::infrastructure::tools::spawn::SpawnTool;

/// Parsed flags for the `agent` subcommand.
pub(crate) struct AgentFlags {
    /// Session name for persistence. `None` = "default", `Some("-")` = ephemeral.
    pub(crate) session_name: Option<String>,
    pub(crate) message: Option<String>,
    pub(crate) system_prompt: Option<String>,
    pub(crate) model_override: Option<String>,
    /// Override max tool iterations (takes precedence over config).
    pub(crate) max_iterations: Option<u32>,
    /// Wall-clock timeout in seconds for the entire agent run.
    pub(crate) max_time: Option<u64>,
}

/// Bundles the stdout/stderr pair passed through the agent pipeline.
pub(crate) struct AgentOutput<'a> {
    pub(crate) stdout: &'a mut String,
    pub(crate) stderr: &'a mut String,
}

/// Outcome of a deadline-bounded agent run.
pub(crate) enum DeadlineResult {
    /// Agent completed (successfully or with error) within the deadline.
    Completed(Result<crate::domain::agent::AgentResult, crate::domain::error::DomainError>),
    /// The deadline expired before the agent finished.
    TimedOut,
}

pub(crate) fn parse_agent_flags(args: &[String], stderr: &mut String) -> Option<AgentFlags> {
    let mut session_name: Option<String> = None;
    let mut message: Option<String> = None;
    let mut system_prompt: Option<String> = None;
    let mut model_override: Option<String> = None;
    let mut max_iterations: Option<u32> = None;
    let mut max_time: Option<u64> = None;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "-s" | "--session" => {
                if i + 1 < args.len() {
                    let name = &args[i + 1];
                    if !super::is_valid_session_name(name) {
                        stderr.push_str(
                            "agent: session name must contain only alphanumeric, '-', or '_'\n",
                        );
                        return None;
                    }
                    session_name = Some(name.clone());
                    i += 2;
                } else {
                    stderr.push_str("agent: -s requires a session name\n");
                    return None;
                }
            }
            "-m" | "--message" => {
                if i + 1 < args.len() {
                    message = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    stderr.push_str("agent: -m requires a message\n");
                    return None;
                }
            }
            "--system" => {
                if i + 1 < args.len() {
                    system_prompt = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    stderr.push_str("agent: --system requires a value\n");
                    return None;
                }
            }
            "--model" => {
                if i + 1 < args.len() {
                    model_override = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    stderr.push_str("agent: --model requires a value\n");
                    return None;
                }
            }
            "--max-iterations" => {
                if i + 1 < args.len() {
                    match args[i + 1].parse::<u32>() {
                        Ok(0) | Err(_) => {
                            stderr
                                .push_str("agent: --max-iterations requires a positive integer\n");
                            return None;
                        }
                        Ok(n) => max_iterations = Some(n),
                    }
                    i += 2;
                } else {
                    stderr.push_str("agent: --max-iterations requires a value\n");
                    return None;
                }
            }
            "--max-time" => {
                if i + 1 < args.len() {
                    match args[i + 1].parse::<u64>() {
                        Ok(0) | Err(_) => {
                            stderr.push_str("agent: --max-time requires a positive integer\n");
                            return None;
                        }
                        Ok(n) => max_time = Some(n),
                    }
                    i += 2;
                } else {
                    stderr.push_str("agent: --max-time requires a value\n");
                    return None;
                }
            }
            _ => {
                i += 1;
            }
        }
    }

    Some(AgentFlags {
        session_name,
        message,
        system_prompt,
        model_override,
        max_iterations,
        max_time,
    })
}

pub(crate) fn cmd_agent(
    ctx: &CliContext,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let mut flags = match parse_agent_flags(args, stderr) {
        Some(f) => f,
        None => return 1,
    };

    if flags.message.is_none() {
        stderr.push_str("agent: -m is required for non-interactive mode\n");
        return 1;
    }

    let base_dir = ctx.base_dir();
    let agent = match build_agent_from_config(&base_dir, &flags, stderr) {
        Some(a) => a,
        None => return 1,
    };

    // Build system prompt: datetime preamble + skills + user prompt
    let skill_prompt = crate::interface::shared::load_skill_prompt(&base_dir);
    flags.system_prompt = Some(crate::interface::shared::build_system_prompt(
        &skill_prompt,
        &flags.system_prompt,
    ));

    let mut out = AgentOutput { stdout, stderr };
    run_agent_session(&base_dir, agent, &flags, &mut out)
}

/// Load config, build provider, and construct the agent loop. Returns None on error.
pub(crate) fn build_agent_from_config(
    base_dir: &std::path::Path,
    flags: &AgentFlags,
    stderr: &mut String,
) -> Option<AgentLoopImpl> {
    let config_path = base_dir.join("config.json");
    if !config_path.exists() {
        stderr.push_str(&format!(
            "config not found at {}\nrun 'quecto onboard' first\n",
            config_path.display()
        ));
        return None;
    }

    let env_overrides: HashMap<String, String> = std::env::vars()
        .filter(|(k, _)| k.starts_with("QUECTO_"))
        .collect();

    let config = match Config::load_with_env(config_path.to_str().unwrap_or(""), &env_overrides) {
        Ok(c) => c,
        Err(e) => {
            stderr.push_str(&format!("failed to load config: {}\n", e));
            return None;
        }
    };

    let provider = match build_agent_provider(&config, base_dir) {
        Ok(p) => p,
        Err(msg) => {
            stderr.push_str(&format!("{}\n", msg));
            return None;
        }
    };

    let workspace = PathBuf::from(config.workspace_path());
    let model = flags
        .model_override
        .clone()
        .unwrap_or(config.agents.defaults.model.clone());
    let sandbox = Sandbox::new(
        Some(workspace.clone()),
        config.agents.defaults.restrict_to_workspace,
    );
    let exec_settings = ToolRegistryImpl::exec_registry_settings_from_config(&config);
    let registry =
        ToolRegistryImpl::with_core_tools_and_exec_settings(workspace, sandbox, exec_settings);
    let mut registry = registry;
    let session_key = if flags.session_name.as_deref() == Some("-") {
        String::new()
    } else {
        let name = flags.session_name.as_deref().unwrap_or("default");
        Session::build_key("cli", name)
    };
    let spill_store = Arc::new(FileContextSpillStore::new(base_dir.to_path_buf()));
    registry.register(Arc::new(RecallTool::new(
        spill_store.clone(),
        session_key.clone(),
    )));
    registry.register(Arc::new(SpawnTool::with_base_dir(
        vec![],
        config.agents.defaults.restrict_to_workspace,
        base_dir.to_path_buf(),
    )));
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(registry),
        model,
        max_tokens: config.agents.defaults.max_tokens,
        temperature: config.agents.defaults.temperature,
        spill_store: Some(spill_store),
        session_key,
        context_collapse_after_turns: config.agents.defaults.context_collapse_after_turns,
        max_context_tokens: config.agents.defaults.max_context_tokens,
        progress_callback: None,
    })
    .with_max_tool_iterations(
        flags
            .max_iterations
            .unwrap_or(config.agents.defaults.max_tool_iterations),
    );

    Some(agent)
}

/// Run the agent loop with session load/save. Returns the exit code.
pub(crate) fn run_agent_session(
    base_dir: &std::path::Path,
    agent: AgentLoopImpl,
    flags: &AgentFlags,
    out: &mut AgentOutput<'_>,
) -> i32 {
    let ephemeral = flags.session_name.as_deref() == Some("-");
    let session_key = if ephemeral {
        String::new()
    } else {
        let name = flags.session_name.as_deref().unwrap_or("default");
        Session::build_key("cli", name)
    };

    let session_store = FileSessionStore::new(base_dir);
    let rt = match super::build_tokio_runtime() {
        Ok(rt) => rt,
        Err(e) => {
            out.stderr
                .push_str(&format!("failed to create runtime: {}\n", e));
            return 1;
        }
    };

    let mut messages: Vec<Message> = if !ephemeral {
        match rt.block_on(session_store.load(&session_key)) {
            Ok(Some(session)) => session.messages,
            Ok(None) => Vec::new(),
            Err(e) => {
                out.stderr
                    .push_str(&format!("failed to load session: {}\n", e));
                return 1;
            }
        }
    } else {
        Vec::new()
    };

    // System prompt is injected at call time but not persisted in session history.
    // Track its index so we can remove exactly this message before saving.
    let system_prompt_idx = if flags.system_prompt.is_some() {
        let idx = messages.len();
        messages.push(Message::system(
            flags.system_prompt.as_deref().unwrap_or("").to_string(),
        ));
        Some(idx)
    } else {
        None
    };

    let message = flags.message.as_deref().unwrap_or("");
    messages.push(Message::user(message.to_string()));

    let agent_result = if let Some(secs) = flags.max_time {
        match run_with_deadline(&rt, &agent, &mut messages, secs) {
            DeadlineResult::Completed(inner) => inner,
            DeadlineResult::TimedOut => {
                out.stderr.push_str("max-time exceeded\n");
                return 2;
            }
        }
    } else {
        rt.block_on(agent.process(&mut messages))
    };

    match agent_result {
        Ok(result) => {
            if !ephemeral {
                if let Some(idx) = system_prompt_idx {
                    if idx < messages.len() {
                        messages.remove(idx);
                    }
                }
                let session = Session {
                    key: session_key,
                    messages: std::mem::take(&mut messages),
                };
                if let Err(e) = rt.block_on(session_store.save(&session)) {
                    out.stderr
                        .push_str(&format!("warning: failed to save session: {}\n", e));
                }
            }
            out.stdout.push_str(&result.response);
            out.stdout.push('\n');
            0
        }
        Err(e) => {
            out.stderr.push_str(&format!("Error: {}\n", e));
            1
        }
    }
}

/// Run the agent with a wall-clock deadline.
///
/// Uses `thread::scope` + `recv_timeout` so the deadline is enforced without
/// requiring `tokio::time::timeout` (which needs an active reactor context
/// that may conflict with test harness runtimes). After timeout, the scoped
/// thread still runs until the in-flight LLM/tool call completes (bounded by
/// per-tool and HTTP client timeouts), then the scope exits.
pub(crate) fn run_with_deadline(
    rt: &tokio::runtime::Runtime,
    agent: &AgentLoopImpl,
    messages: &mut Vec<Message>,
    timeout_secs: u64,
) -> DeadlineResult {
    let dur = std::time::Duration::from_secs(timeout_secs);
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let deadline = std::time::Instant::now() + dur;

    std::thread::scope(|s| {
        s.spawn(|| {
            let result = rt.block_on(agent.process(messages));
            let _ = tx.send(result);
        });

        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match rx.recv_timeout(remaining) {
            Ok(result) => DeadlineResult::Completed(result),
            Err(_) => DeadlineResult::TimedOut,
        }
    })
}

/// Build a FallbackProvider from config + credential store, suitable for the agent CLI.
pub fn build_agent_provider(
    config: &Config,
    base_dir: &std::path::Path,
) -> Result<Arc<dyn LlmProvider>, String> {
    let store = CredentialStore::new(base_dir);

    // Build a temporary runtime for token refresh if needed
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("failed to create runtime for token refresh: {}", e))?;

    let mut provider_list: Vec<Arc<dyn crate::domain::provider::LlmProvider>> = Vec::new();

    // Try OpenAI (with auto-refresh for expired OAuth tokens)
    let openai_key = crate::interface::shared::resolve_api_key_with_refresh(
        &config.providers.openai.api_key,
        &store,
        "openai",
        &rt,
    );
    if !openai_key.is_empty() {
        let account_id = crate::infrastructure::auth::oauth::extract_openai_account_id(&openai_key);
        if let Some(acct) = account_id {
            // OAuth token — use Codex provider (ChatGPT backend)
            provider_list.push(providers::create_codex_provider(openai_key, acct));
        } else {
            // Regular API key — use standard OpenAI provider
            let base = if config.providers.openai.api_base.is_empty() {
                None
            } else {
                Some(config.providers.openai.api_base.clone())
            };
            match providers::create_provider("openai", openai_key, base) {
                Ok(p) => provider_list.push(p),
                Err(e) => {
                    return Err(format!("openai provider configuration error: {}", e));
                }
            }
        }
    }

    // Try Anthropic (with auto-refresh for expired OAuth tokens)
    let anthropic_key = crate::interface::shared::resolve_api_key_with_refresh(
        &config.providers.anthropic.api_key,
        &store,
        "anthropic",
        &rt,
    );
    if !anthropic_key.is_empty() {
        let base = if config.providers.anthropic.api_base.is_empty() {
            None
        } else {
            Some(config.providers.anthropic.api_base.clone())
        };
        match providers::create_provider("anthropic", anthropic_key, base) {
            Ok(p) => provider_list.push(p),
            Err(e) => {
                return Err(format!("anthropic provider configuration error: {}", e));
            }
        }
    }

    if provider_list.is_empty() {
        return Err(
            "no LLM providers configured (set an API key or run 'quecto auth login')".to_string(),
        );
    }

    Ok(Arc::new(FallbackProvider::new(provider_list)))
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "agent_integration_tests.rs"]
mod integration_tests;
