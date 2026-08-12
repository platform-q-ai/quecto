mod cmd_agent;
mod cmd_spawn;
mod parsers;
pub(crate) mod progress;

use std::io::{BufRead, Write};
use std::path::Path;
use std::sync::Arc;

use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::domain::agent::AgentLoop;
use crate::domain::error::DomainError;
use crate::domain::message::{Message, Role};
use crate::domain::provider::LlmProvider;
use crate::domain::session::{Session, SessionStore};
use crate::infrastructure::config::Config;
use crate::infrastructure::persistence::session_store::FileSessionStore;
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::registry::ToolRegistryImpl;

use std::path::PathBuf;

/// Parsed flags that apply to REPL mode.
///
/// # Security note
///
/// The `no_sandbox` field disables the workspace path restriction that confines
/// all filesystem tools to the configured workspace directory. Setting it to
/// `true` allows the agent to read and write **any path on the system**.
/// This is intentional when running quecto as a coding assistant on an arbitrary
/// repo, but must never be set implicitly or without user consent.
pub struct ReplFlags {
    pub session_name: Option<String>,
    pub system_prompt: Option<String>,
    pub model_override: Option<String>,
    /// When true, disable workspace path restriction for all filesystem tools.
    /// Overrides `config.agents.defaults.restrict_to_workspace`.
    /// WARNING: allows the agent to read/write any path on the system.
    pub no_sandbox: bool,
}

/// Session state for the REPL (agent, persistence, history).
struct ReplSession {
    agent: AgentLoopImpl,
    messages: Vec<Message>,
    session_store: FileSessionStore,
    session_key: String,
    ephemeral: bool,
    system_prompt: Option<String>,
    /// Base directory for accessing config, etc.
    base_dir: PathBuf,
}

/// REPL loop that reads from any `BufRead` and writes to any `Write`.
///
/// This abstraction allows the REPL to be driven by:
/// - Real stdin/stdout (interactive terminal use)
/// - In-memory buffers (BDD testing)
/// - Quectoped input (scripting: `echo "hello" | quecto`)
pub struct ReplLoop<R: BufRead, W: Write> {
    reader: R,
    writer: W,
    is_tty: bool,
    session: ReplSession,
}

/// REPL slash commands.
const CMD_EXIT: &str = "/exit";
const CMD_QUIT: &str = "/quit";
const CMD_HELP: &str = "/help";
const CMD_CLEAR: &str = "/clear";
const CMD_AGENT: &str = "/agent";
const CMD_SPAWN: &str = "/spawn";

impl<R: BufRead, W: Write> ReplLoop<R, W> {
    /// Create a new REPL loop.
    fn new(reader: R, writer: W, is_tty: bool, session: ReplSession) -> Self {
        Self {
            reader,
            writer,
            is_tty,
            session,
        }
    }

    /// Run the REPL loop. Returns the exit code.
    fn run(&mut self) -> i32 {
        let rt = match build_repl_runtime() {
            Ok(rt) => rt,
            Err(e) => {
                let _ = writeln!(self.writer, "Error: failed to create runtime: {e}");
                return 1;
            }
        };

        if self.is_tty {
            self.print_banner();
        }

        let mut line = String::new();
        loop {
            if self.is_tty {
                let _ = write!(self.writer, "> ");
                let _ = self.writer.flush();
            }

            line.clear();
            match self.reader.read_line(&mut line) {
                Ok(0) => break, // EOF
                Ok(_) => {}
                Err(e) => {
                    let _ = writeln!(self.writer, "Error reading input: {e}");
                    break;
                }
            }

            let input = line.trim();
            if input.is_empty() {
                continue;
            }

            match input {
                CMD_EXIT | CMD_QUIT => break,
                CMD_HELP => {
                    self.print_help();
                    continue;
                }
                CMD_CLEAR => {
                    self.handle_clear(&rt);
                    continue;
                }
                _ if input.starts_with(CMD_AGENT) => {
                    self.handle_agent(input, &rt);
                    continue;
                }
                _ if input.starts_with(CMD_SPAWN) => {
                    self.handle_spawn(input, &rt);
                    continue;
                }
                _ => {}
            }

            self.process_input(&rt, input);
        }

        self.save_session_on_exit(&rt);
        0
    }

    fn print_banner(&mut self) {
        let version = env!("CARGO_PKG_VERSION");
        let _ = writeln!(self.writer, "quecto v{version} — Interactive Mode");
        let _ = writeln!(self.writer, "Type /help for commands, /exit to quit");
        let _ = writeln!(self.writer);
    }

    fn print_help(&mut self) {
        let _ = writeln!(self.writer, "Commands:");
        let _ = writeln!(self.writer, "  /help       Show this help");
        let _ = writeln!(self.writer, "  /clear      Clear conversation history");
        let _ = writeln!(self.writer, "  /agent      Manage subagent profiles");
        let _ = writeln!(self.writer, "  /spawn      Spawn a task as a child agent");
        let _ = writeln!(self.writer, "  /exit       Exit the REPL");
        let _ = writeln!(self.writer, "  /quit       Exit the REPL");
    }

    fn handle_clear(&mut self, rt: &tokio::runtime::Runtime) {
        self.session.messages.clear();
        if !self.session.ephemeral {
            let session = Session {
                key: self.session.session_key.clone(),
                messages: Vec::new(),
                workflow_run: None,
                subagent_roster: Vec::new(),
            };
            if let Err(e) = rt.block_on(self.session.session_store.save(&session)) {
                let _ = writeln!(self.writer, "Warning: failed to clear session: {e}");
            }
        }
        let _ = writeln!(self.writer, "Conversation cleared.");
    }

    // -----------------------------------------------------------------------
    // Agent input processing
    // -----------------------------------------------------------------------

    fn process_input(&mut self, rt: &tokio::runtime::Runtime, input: &str) {
        let system_idx = self.inject_system_prompt();

        self.session.messages.push(Message::user(input.to_string()));

        let result = rt.block_on(self.session.agent.process(&mut self.session.messages));

        // Remove the system prompt by matching role + content, not by index.
        // This is safe even if process() inserts messages before the system prompt position.
        self.remove_system_prompt(system_idx);

        match result {
            Ok(r) => {
                let _ = writeln!(self.writer, "{}", r.response);
            }
            Err(e) => {
                let _ = writeln!(self.writer, "Error: {e}");
            }
        }
    }

    fn inject_system_prompt(&mut self) -> Option<usize> {
        self.session.system_prompt.as_ref().map(|prompt| {
            let idx = self.session.messages.len();
            self.session.messages.push(Message::system(prompt.clone()));
            idx
        })
    }

    /// Remove the system prompt injected at `idx`.
    ///
    /// Scans backwards from `idx` to find the system message, in case the
    /// agent loop inserted messages before it (defensive). Falls back to
    /// forward scan if not found.
    fn remove_system_prompt(&mut self, idx: Option<usize>) {
        let Some(original_idx) = idx else { return };
        let Some(prompt) = &self.session.system_prompt else {
            return;
        };

        // Try the original index first (fast path).
        if original_idx < self.session.messages.len() {
            let msg = &self.session.messages[original_idx];
            if msg.role == Role::System && msg.content == *prompt {
                self.session.messages.remove(original_idx);
                return;
            }
        }

        // Fallback: scan for the system message by content.
        if let Some(pos) = self
            .session
            .messages
            .iter()
            .position(|m| m.role == Role::System && m.content == *prompt)
        {
            self.session.messages.remove(pos);
        }
    }

    fn save_session_on_exit(&mut self, rt: &tokio::runtime::Runtime) {
        if !self.session.ephemeral {
            let session = Session {
                key: self.session.session_key.clone(),
                messages: self.session.messages.clone(),
                workflow_run: None,
                subagent_roster: Vec::new(),
            };
            if let Err(e) = rt.block_on(self.session.session_store.save(&session)) {
                let _ = writeln!(self.writer, "Warning: failed to save session: {e}");
            }
        }
    }
}

/// Build a tokio runtime for REPL execution.
fn build_repl_runtime() -> Result<tokio::runtime::Runtime, std::io::Error> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
}

/// Build and run the REPL with the given I/O, config, and flags.
/// This is the main entry point called from cli.rs.
pub fn run_repl<R: BufRead, W: Write>(
    reader: R,
    mut writer: W,
    is_tty: bool,
    ctx: &ReplContext<'_>,
) -> i32 {
    let workspace = crate::interface::shared::resolve_agent_workspace(
        &ctx.config.workspace_path(),
        ctx.flags.no_sandbox,
    );
    let model = ctx
        .flags
        .model_override
        .clone()
        .unwrap_or(ctx.config.agents.defaults.model.clone());
    // --no-sandbox overrides config: disables workspace path restriction for all
    // filesystem tools. The dangerous-command denylist remains active regardless.
    if ctx.flags.no_sandbox {
        tracing::warn!("--no-sandbox: workspace path restriction disabled");
    }
    let sandbox = Sandbox::for_agent_workspace(ctx.config, workspace.clone(), ctx.flags.no_sandbox);
    let exec_settings = ToolRegistryImpl::exec_registry_settings_from_config(ctx.config);
    let exec_options = crate::infrastructure::tools::bash::ExecOptions {
        max_capture_bytes: exec_settings,
        ..crate::infrastructure::tools::bash::ExecOptions::default()
    };
    let ephemeral = ctx.flags.session_name.as_deref() == Some("-");
    let session_key = if ephemeral {
        String::new()
    } else if let Some(name) = ctx.flags.session_name.as_deref() {
        Session::build_key("repl", name)
    } else {
        crate::interface::shared::generate_chat_key()
    };
    let mut stderr = String::new();
    let runtime = match crate::interface::shared::build_tool_runtime(
        crate::interface::shared::ToolRuntimeBuildArgs {
            entrypoint: crate::interface::shared::ToolEntrypoint::Repl,
            profile_context: crate::interface::tool_runtime::ToolRuntimeProfileContext::Parent,
            base_dir: ctx.base_dir,
            config: ctx.config,
            http_client: &crate::interface::shared::build_http_client(),
            workspace,
            sandbox,
            exec_options,
            session_key,
            spawned: false,
            restrict_to_workspace: !ctx.flags.no_sandbox
                && ctx.config.agents.defaults.restrict_to_workspace,
            parent_session_name: ctx.flags.session_name.clone(),
            parent_config_path: Some(ctx.config_path.to_path_buf()),
            disabled_tools: &[],
            inherited_tool_policy: None,
            workflow: crate::interface::shared::ToolRuntimeWorkflowPolicy::disabled(
                ctx.base_dir,
                None,
            ),
            stderr: &mut stderr,
        },
    ) {
        Ok(runtime) => runtime,
        Err(err) => {
            let _ = writeln!(writer, "Error: failed to initialize tool runtime: {err}");
            return 1;
        }
    };
    if !stderr.is_empty() {
        tracing::warn!("{stderr}");
    }
    let _policy_state = runtime.policy_state;
    let _catalogue_entries = runtime.catalogue_entries;
    let registry = runtime.registry;
    let spill_store = runtime.spill_store;
    let session_key = runtime.session_key;

    // Resolve the progress callback:
    // 1. If the caller injected an explicit callback (e.g. BDD test recorder), use it.
    // 2. If this is a real TTY session, spawn the background spinner thread.
    // 3. Otherwise (non-TTY pipe/redirect), use None (silent).
    let (progress_callback, spinner_handle) = resolve_progress_callback(ctx, is_tty);

    // #935: clamp the effective output cap to the model's registry max_tokens so
    // a model whose real output limit is lower than the configured global
    // default (e.g. Fireworks qwen3p7-plus = 65536) never receives a larger
    // value. Mirror the CLI build path (interface/cli/agent.rs) so the REPL does
    // not silently bypass the clamp.
    // #1044: the model's known window bounds the pruning budget; one registry
    // load supplies both per-model limits.
    let (model_max_tokens, model_context_window) =
        crate::infrastructure::model_registry::ModelRegistry::model_limits_from_base_dir(
            ctx.base_dir,
            &model,
        );
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: ctx.provider.clone(),
        tool_registry: Box::new(registry),
        model,
        max_tokens: ctx.config.agents.defaults.max_tokens,
        temperature: ctx.config.agents.defaults.temperature,
        spill_store: Some(spill_store),
        session_key: session_key.clone(),
        context_collapse_after_tool_calls: ctx
            .config
            .agents
            .defaults
            .context_collapse_after_tool_calls,
        max_context_tokens: ctx.config.agents.defaults.max_context_tokens,
        progress_callback,
        streaming: false,
        effort: resolve_effort_from_config(ctx.config),
        audit_log: None,
        // #1044/#1045/#1046: the context knobs are constructor fields so this
        // site cannot silently drop the user's configured values.
        pin_recent_turns: ctx.config.agents.defaults.pin_recent_turns,
        context_collapse_after_messages: ctx.config.agents.defaults.context_collapse_after_messages,
        model_context_window,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    })
    .with_model_max_tokens(model_max_tokens);

    let session_store = FileSessionStore::new(ctx.base_dir);

    // Create the runtime once and reuse for both session loading and the REPL loop.
    let rt = match build_repl_runtime() {
        Ok(rt) => rt,
        Err(e) => {
            // Can't proceed without a runtime — fall back to empty messages.
            tracing::error!("failed to create runtime for session load: {e}");
            let session = ReplSession {
                agent,
                messages: Vec::new(),
                session_store,
                session_key,
                ephemeral,
                system_prompt: build_system_prompt(ctx),
                base_dir: ctx.base_dir.to_path_buf(),
            };
            let code = ReplLoop::new(reader, writer, is_tty, session).run();
            if let Some(handle) = spinner_handle {
                handle.stop();
            }
            crate::interface::shared::scrub_ephemeral_spill(ctx.base_dir, ephemeral);
            return code;
        }
    };

    let mut messages =
        match load_session_messages_with_rt(&rt, &session_store, &session_key, ephemeral) {
            Ok(messages) => messages,
            Err(e) => {
                let _ = writeln!(writer, "Error: {e}");
                if let Some(handle) = spinner_handle {
                    handle.stop();
                }
                crate::interface::shared::scrub_ephemeral_spill(ctx.base_dir, ephemeral);
                return 1;
            }
        };
    if !ephemeral && !messages.is_empty() {
        rt.block_on(agent.prune_resumed_context(&mut messages));
    }

    let session = ReplSession {
        agent,
        messages,
        session_store,
        session_key,
        ephemeral,
        system_prompt: build_system_prompt(ctx),
        base_dir: ctx.base_dir.to_path_buf(),
    };

    // Drop the pre-built runtime so ReplLoop::run() can create its own
    // (current_thread runtimes cannot be nested).
    drop(rt);
    let code = ReplLoop::new(reader, writer, is_tty, session).run();
    // Stop the spinner thread cleanly — it clears the last spinner line before exiting.
    if let Some(handle) = spinner_handle {
        handle.stop();
    }
    // An ephemeral (`-s -`) REPL persisted spill content only for in-run recall.
    crate::interface::shared::scrub_ephemeral_spill(ctx.base_dir, ephemeral);
    code
}

/// Resolve which progress callback to use for this REPL session.
///
/// Priority:
/// 1. An explicit callback in `ctx.progress_callback` (BDD test recorder).
/// 2. A freshly spawned spinner thread when `is_tty = true` (interactive terminal).
/// 3. `None` when `is_tty = false` (pipe / redirect / non-interactive).
///
/// Returns `(callback, Option<SpinnerHandle>)`. The caller must call
/// `SpinnerHandle::stop()` when the REPL loop exits so the spinner thread is
/// cleanly joined and the terminal line is erased.
///
/// ### Thread lifetime
///
/// The spinner thread lives for the entire REPL session, including idle time
/// between user inputs. The thread blocks on `mpsc::recv_timeout(80ms)` and
/// consumes negligible CPU when no events are flowing. On resource-constrained
/// targets (RQuecto, containers) the idle cost is ~1 wakeup/80ms — acceptable for
/// an interactive terminal tool. Future optimization: spawn per `process_input()`
/// call and stop immediately after if tighter resource bounds are needed.
fn resolve_progress_callback(
    ctx: &ReplContext<'_>,
    is_tty: bool,
) -> (
    Option<crate::domain::agent::ProgressCallback>,
    Option<progress::SpinnerHandle>,
) {
    // Explicit callback takes precedence (e.g. injected by BDD test recorder).
    if let Some(cb) = ctx.progress_callback.clone() {
        return (Some(cb), None);
    }
    // TTY session: spawn the live spinner thread.
    if is_tty {
        let status_header = progress::build_status_header_line();
        let (cb, handle) = progress::spawn_spinner_thread_with_status(status_header);
        return (Some(cb), Some(handle));
    }
    // Non-TTY: no progress output.
    (None, None)
}

/// Resolve effort level from config for the REPL path.
///
/// CLI `--effort` flag is not available in REPL mode; effort is set
/// via `agents.defaults.effort` in config or the `QUECTO_AGENTS_DEFAULTS_EFFORT`
/// env var.
fn resolve_effort_from_config(config: &Config) -> Option<crate::domain::provider::EffortLevel> {
    config.agents.defaults.effort.as_deref().and_then(|s| {
        crate::domain::provider::EffortLevel::parse(s).or_else(|| {
            // Unreachable via config/env — Config::load rejects unknown
            // effort values at load time (#1066); defensive fallback only.
            eprintln!(
                "WARNING: invalid effort level '{}' in config; expected one of: {}; ignoring",
                s,
                crate::domain::provider::EffortLevel::VALID_VALUES
            );
            None
        })
    })
}

/// Build the system prompt from the docs retrieval policy plus optional user prompt.
fn build_system_prompt(ctx: &ReplContext<'_>) -> Option<String> {
    // REPL is always a top-level interactive parent agent (#1319).
    Some(super::shared::build_system_prompt(
        &ctx.flags.system_prompt,
        false,
    ))
}

/// Context for constructing a REPL session.
pub struct ReplContext<'a> {
    pub base_dir: &'a Path,
    /// The path the session's config was loaded from — plumbed into SpawnTool
    /// as the container-config fallback (#1369 follow-up).
    pub config_path: &'a Path,
    pub provider: Arc<dyn LlmProvider>,
    pub config: &'a Config,
    pub flags: &'a ReplFlags,
    /// Optional progress callback injected by the caller (e.g. BDD test recorder
    /// or the live TTY spinner). When `None`, the REPL builds its own spinner
    /// for TTY sessions or skips progress reporting for non-TTY.
    pub progress_callback: Option<crate::domain::agent::ProgressCallback>,
}

/// Load existing session messages using a provided runtime.
fn load_session_messages_with_rt(
    rt: &tokio::runtime::Runtime,
    store: &FileSessionStore,
    key: &str,
    ephemeral: bool,
) -> Result<Vec<Message>, DomainError> {
    if ephemeral {
        return Ok(Vec::new());
    }
    store.claim(key)?;
    match rt.block_on(store.load(key))? {
        Some(session) => Ok(session.messages),
        None => Ok(Vec::new()),
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "mod_cov_tests.rs"]
mod cov_tests;
