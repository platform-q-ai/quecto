use std::io::{BufRead, Write};
use std::path::Path;
use std::sync::Arc;

use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::domain::agent::AgentLoop;
use crate::domain::cron::{CronJob, CronSchedule, CronStore};
use crate::domain::message::{Message, Role};
use crate::domain::provider::LlmProvider;
use crate::domain::session::{Session, SessionStore};
use crate::infrastructure::config::Config;
use crate::infrastructure::persistence::cron_store::FileCronStore;
use crate::infrastructure::persistence::session_store::FileSessionStore;
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::registry::ToolRegistryImpl;

use std::path::PathBuf;

/// Parsed flags that apply to REPL mode.
pub struct ReplFlags {
    pub session_name: Option<String>,
    pub system_prompt: Option<String>,
    pub model_override: Option<String>,
}

/// Session state for the REPL (agent, persistence, history).
struct ReplSession {
    agent: AgentLoopImpl,
    messages: Vec<Message>,
    session_store: FileSessionStore,
    session_key: String,
    ephemeral: bool,
    system_prompt: Option<String>,
    /// Base directory for accessing cron store, config, etc.
    base_dir: PathBuf,
}

/// REPL loop that reads from any `BufRead` and writes to any `Write`.
///
/// This abstraction allows the REPL to be driven by:
/// - Real stdin/stdout (interactive terminal use)
/// - In-memory buffers (BDD testing)
/// - Piped input (scripting: `echo "hello" | quecto`)
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
const CMD_CRON: &str = "/cron";

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
                _ if input.starts_with(CMD_CRON) => {
                    self.handle_cron(input);
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
        let _ = writeln!(self.writer, "  /cron       Manage scheduled cron jobs");
        let _ = writeln!(self.writer, "  /exit       Exit the REPL");
        let _ = writeln!(self.writer, "  /quit       Exit the REPL");
    }

    fn handle_clear(&mut self, rt: &tokio::runtime::Runtime) {
        self.session.messages.clear();
        if !self.session.ephemeral {
            let session = Session {
                key: self.session.session_key.clone(),
                messages: Vec::new(),
            };
            if let Err(e) = rt.block_on(self.session.session_store.save(&session)) {
                let _ = writeln!(self.writer, "Warning: failed to clear session: {e}");
            }
        }
        let _ = writeln!(self.writer, "Conversation cleared.");
    }

    // -----------------------------------------------------------------------
    // /cron command
    // -----------------------------------------------------------------------

    fn handle_cron(&mut self, input: &str) {
        let rest = input.strip_prefix(CMD_CRON).unwrap_or("").trim();
        let parts: Vec<&str> = rest.splitn(2, char::is_whitespace).collect();
        let subcmd = parts.first().copied().unwrap_or("");
        let args_str = if parts.len() > 1 { parts[1] } else { "" };

        match subcmd {
            "list" => self.cron_list(),
            "add" => self.cron_add(args_str),
            "remove" => self.cron_remove(args_str),
            "enable" => self.cron_enable(args_str),
            "disable" => self.cron_disable(args_str),
            _ => self.cron_usage(),
        }
    }

    fn cron_store(&self) -> FileCronStore {
        FileCronStore::new(&self.session.base_dir)
    }

    fn cron_usage(&mut self) {
        let _ = writeln!(self.writer, "Usage: /cron <subcommand>");
        let _ = writeln!(self.writer, "  add      Add a new cron job");
        let _ = writeln!(self.writer, "  list     List all cron jobs");
        let _ = writeln!(self.writer, "  remove   Remove a cron job");
        let _ = writeln!(self.writer, "  enable   Enable a cron job");
        let _ = writeln!(self.writer, "  disable  Disable a cron job");
    }

    fn cron_list(&mut self) {
        let store = self.cron_store();
        match store.list() {
            Ok(jobs) if jobs.is_empty() => {
                let _ = writeln!(self.writer, "No scheduled jobs");
            }
            Ok(jobs) => {
                let _ = writeln!(self.writer, "Scheduled jobs:");
                for job in &jobs {
                    let schedule_str = match &job.schedule {
                        CronSchedule::Interval { seconds } => format!("every {}s", seconds),
                        CronSchedule::Cron { expression } => format!("cron: {}", expression),
                    };
                    let status = if job.enabled { "enabled" } else { "disabled" };
                    let _ = writeln!(
                        self.writer,
                        "  {} — {} [{}]",
                        job.name, schedule_str, status
                    );
                }
            }
            Err(e) => {
                let _ = writeln!(self.writer, "Error: {}", e);
            }
        }
    }

    fn cron_add(&mut self, args_str: &str) {
        match parse_cron_add_args(args_str) {
            Ok(parsed) => {
                let store = self.cron_store();
                // Check for duplicate name
                match store.find_by_name(&parsed.name) {
                    Ok(Some(_)) => {
                        let _ =
                            writeln!(self.writer, "Error: job '{}' already exists", parsed.name);
                        return;
                    }
                    Err(e) => {
                        let _ = writeln!(self.writer, "Error: {}", e);
                        return;
                    }
                    _ => {}
                }
                let job = CronJob {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: parsed.name.clone(),
                    message: parsed.message,
                    schedule: parsed.schedule,
                    enabled: true,
                    deliver_to: parsed.deliver_to,
                    last_error: None,
                    last_run_at: 0,
                };
                match store.add(job) {
                    Ok(()) => {
                        let _ = writeln!(self.writer, "Job '{}' created", parsed.name);
                    }
                    Err(e) => {
                        let _ = writeln!(self.writer, "Error: {}", e);
                    }
                }
            }
            Err(msg) => {
                let _ = writeln!(self.writer, "Error: {}", msg);
            }
        }
    }

    fn cron_remove(&mut self, args_str: &str) {
        let name = args_str.trim();
        if name.is_empty() {
            let _ = writeln!(self.writer, "Error: missing job name");
            return;
        }
        let store = self.cron_store();
        match store.find_by_name(name) {
            Ok(Some(job)) => match store.remove(&job.id) {
                Ok(()) => {
                    let _ = writeln!(self.writer, "Job '{}' removed", name);
                }
                Err(e) => {
                    let _ = writeln!(self.writer, "Error: {}", e);
                }
            },
            Ok(None) => {
                let _ = writeln!(self.writer, "Error: job '{}' not found", name);
            }
            Err(e) => {
                let _ = writeln!(self.writer, "Error: {}", e);
            }
        }
    }

    fn cron_enable(&mut self, args_str: &str) {
        self.cron_set_enabled(args_str.trim(), true);
    }

    fn cron_disable(&mut self, args_str: &str) {
        self.cron_set_enabled(args_str.trim(), false);
    }

    fn cron_set_enabled(&mut self, name: &str, enabled: bool) {
        if name.is_empty() {
            let _ = writeln!(self.writer, "Error: missing job name");
            return;
        }
        let store = self.cron_store();
        match store.find_by_name(name) {
            Ok(Some(job)) => {
                let action = if enabled { "enabled" } else { "disabled" };
                match store.set_enabled(&job.id, enabled) {
                    Ok(()) => {
                        let _ = writeln!(self.writer, "Job '{}' {}", name, action);
                    }
                    Err(e) => {
                        let _ = writeln!(self.writer, "Error: {}", e);
                    }
                }
            }
            Ok(None) => {
                let _ = writeln!(self.writer, "Error: job '{}' not found", name);
            }
            Err(e) => {
                let _ = writeln!(self.writer, "Error: {}", e);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Agent input processing
    // -----------------------------------------------------------------------

    fn process_input(&mut self, rt: &tokio::runtime::Runtime, input: &str) {
        let system_idx = self.inject_system_prompt();

        self.session.messages.push(Message {
            role: Role::User,
            content: input.to_string(),
            tool_calls: vec![],
            tool_call_id: None,
        });

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
            self.session.messages.push(Message {
                role: Role::System,
                content: prompt.clone(),
                tool_calls: vec![],
                tool_call_id: None,
            });
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
            };
            if let Err(e) = rt.block_on(self.session.session_store.save(&session)) {
                let _ = writeln!(self.writer, "Warning: failed to save session: {e}");
            }
        }
    }
}

// ===========================================================================
// /cron add argument parser
// ===========================================================================

#[derive(Debug)]
struct ParsedCronAdd {
    name: String,
    message: String,
    schedule: CronSchedule,
    deliver_to: Option<String>,
}

/// Parse `/cron add <name> --interval N --message ... [--deliver-to ...] [--cron ...]`
///
/// Uses simple token-based parsing that handles single-quoted values.
fn parse_cron_add_args(args_str: &str) -> Result<ParsedCronAdd, String> {
    let tokens = shell_split_repl(args_str);
    if tokens.is_empty() {
        return Err("missing job name".to_string());
    }

    let name = tokens[0].clone();
    let mut message: Option<String> = None;
    let mut interval: Option<u64> = None;
    let mut cron_expr: Option<String> = None;
    let mut deliver_to: Option<String> = None;

    let mut i = 1;
    while i < tokens.len() {
        match tokens[i].as_str() {
            "--interval" => {
                if i + 1 < tokens.len() {
                    interval = Some(
                        tokens[i + 1]
                            .parse::<u64>()
                            .map_err(|_| "invalid interval value".to_string())?,
                    );
                    i += 2;
                } else {
                    return Err("--interval requires a value".to_string());
                }
            }
            "--cron" => {
                if i + 1 < tokens.len() {
                    cron_expr = Some(tokens[i + 1].clone());
                    i += 2;
                } else {
                    return Err("--cron requires a value".to_string());
                }
            }
            "--message" => {
                if i + 1 < tokens.len() {
                    // Collect all remaining tokens that aren't flags as the message
                    let mut msg_parts = Vec::new();
                    i += 1;
                    while i < tokens.len() && !tokens[i].starts_with("--") {
                        msg_parts.push(tokens[i].clone());
                        i += 1;
                    }
                    if msg_parts.is_empty() {
                        return Err("--message requires a value".to_string());
                    }
                    message = Some(msg_parts.join(" "));
                } else {
                    return Err("--message requires a value".to_string());
                }
            }
            "--deliver-to" => {
                if i + 1 < tokens.len() {
                    deliver_to = Some(tokens[i + 1].clone());
                    i += 2;
                } else {
                    return Err("--deliver-to requires a value".to_string());
                }
            }
            _ => {
                i += 1;
            }
        }
    }

    let message = message.ok_or_else(|| "missing required flag: --message".to_string())?;
    let schedule = match (interval, cron_expr) {
        (Some(seconds), _) => CronSchedule::Interval { seconds },
        (None, Some(expression)) => CronSchedule::Cron { expression },
        (None, None) => {
            return Err("missing schedule: specify --interval or --cron".to_string());
        }
    };

    Ok(ParsedCronAdd {
        name,
        message,
        schedule,
        deliver_to,
    })
}

/// Simple shell-like token splitter for REPL command arguments.
///
/// Handles single-quoted and double-quoted strings. Does not handle
/// backslash escapes (sufficient for REPL slash command parsing).
fn shell_split_repl(s: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b' ' {
            i += 1;
            continue;
        }
        let mut current = String::new();
        if bytes[i] == b'\'' || bytes[i] == b'"' {
            let quote = bytes[i];
            i += 1;
            while i < bytes.len() && bytes[i] != quote {
                current.push(bytes[i] as char);
                i += 1;
            }
            if i < bytes.len() {
                i += 1;
            }
        } else {
            while i < bytes.len() && bytes[i] != b' ' && bytes[i] != b'\'' && bytes[i] != b'"' {
                current.push(bytes[i] as char);
                i += 1;
            }
        }
        if !current.is_empty() {
            tokens.push(current);
        }
    }
    tokens
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
    writer: W,
    is_tty: bool,
    ctx: &ReplContext<'_>,
) -> i32 {
    let workspace = PathBuf::from(ctx.config.workspace_path());
    let model = ctx
        .flags
        .model_override
        .clone()
        .unwrap_or(ctx.config.agents.defaults.model.clone());
    let sandbox = Sandbox::new(
        Some(workspace.clone()),
        ctx.config.agents.defaults.restrict_to_workspace,
    );
    let registry = ToolRegistryImpl::with_core_tools(workspace, sandbox);
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: ctx.provider.clone(),
        tool_registry: Box::new(registry),
        model,
        max_tokens: ctx.config.agents.defaults.max_tokens,
        temperature: ctx.config.agents.defaults.temperature,
    });

    let ephemeral = ctx.flags.session_name.as_deref() == Some("-");
    let session_key = if ephemeral {
        String::new()
    } else {
        let name = ctx.flags.session_name.as_deref().unwrap_or("repl_default");
        Session::build_key("repl", name)
    };

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
            return ReplLoop::new(reader, writer, is_tty, session).run();
        }
    };

    let messages = load_session_messages_with_rt(&rt, &session_store, &session_key, ephemeral);

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
    ReplLoop::new(reader, writer, is_tty, session).run()
}

/// Build the system prompt by loading skills and merging with user prompt.
fn build_system_prompt(ctx: &ReplContext<'_>) -> Option<String> {
    let skill_prompt = super::shared::load_skill_prompt(ctx.base_dir);
    if skill_prompt.is_empty() {
        ctx.flags.system_prompt.clone()
    } else {
        Some(super::shared::merge_prompts(
            &skill_prompt,
            &ctx.flags.system_prompt,
        ))
    }
}

/// Context for constructing a REPL session.
pub struct ReplContext<'a> {
    pub base_dir: &'a Path,
    pub provider: Arc<dyn LlmProvider>,
    pub config: &'a Config,
    pub flags: &'a ReplFlags,
}

/// Load existing session messages using a provided runtime.
fn load_session_messages_with_rt(
    rt: &tokio::runtime::Runtime,
    store: &FileSessionStore,
    key: &str,
    ephemeral: bool,
) -> Vec<Message> {
    if ephemeral {
        return Vec::new();
    }
    match rt.block_on(store.load(key)) {
        Ok(Some(session)) => session.messages,
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slash_command_constants() {
        assert_eq!(CMD_EXIT, "/exit");
        assert_eq!(CMD_QUIT, "/quit");
        assert_eq!(CMD_HELP, "/help");
        assert_eq!(CMD_CLEAR, "/clear");
    }

    #[test]
    fn test_repl_flags_default() {
        let flags = ReplFlags {
            session_name: None,
            system_prompt: None,
            model_override: None,
        };
        assert!(flags.session_name.is_none());
        assert!(flags.system_prompt.is_none());
        assert!(flags.model_override.is_none());
    }

    #[test]
    fn test_repl_flags_with_values() {
        let flags = ReplFlags {
            session_name: Some("mysession".to_string()),
            system_prompt: Some("You are helpful".to_string()),
            model_override: Some("gpt-5-mini".to_string()),
        };
        assert_eq!(flags.session_name.as_deref(), Some("mysession"));
        assert_eq!(flags.system_prompt.as_deref(), Some("You are helpful"));
        assert_eq!(flags.model_override.as_deref(), Some("gpt-5-mini"));
    }

    // -- /cron add parser tests --

    #[test]
    fn test_parse_cron_add_interval() {
        let parsed =
            parse_cron_add_args("weather --interval 3600 --message Check the weather").unwrap();
        assert_eq!(parsed.name, "weather");
        assert_eq!(parsed.message, "Check the weather");
        assert!(matches!(
            parsed.schedule,
            CronSchedule::Interval { seconds: 3600 }
        ));
        assert!(parsed.deliver_to.is_none());
    }

    #[test]
    fn test_parse_cron_add_cron_expression() {
        let parsed =
            parse_cron_add_args("morning-brief --cron '0 9 * * *' --message Good morning brief")
                .unwrap();
        assert_eq!(parsed.name, "morning-brief");
        assert_eq!(parsed.message, "Good morning brief");
        match &parsed.schedule {
            CronSchedule::Cron { expression } => assert_eq!(expression, "0 9 * * *"),
            _ => panic!("expected cron schedule"),
        }
    }

    #[test]
    fn test_parse_cron_add_with_deliver_to() {
        let parsed = parse_cron_add_args(
            "report --interval 86400 --message Daily report --deliver-to telegram:12345",
        )
        .unwrap();
        assert_eq!(parsed.name, "report");
        assert_eq!(parsed.deliver_to.as_deref(), Some("telegram:12345"));
    }

    #[test]
    fn test_parse_cron_add_missing_message() {
        let err = parse_cron_add_args("bad-job --interval 60").unwrap_err();
        assert!(err.contains("missing required flag: --message"), "{}", err);
    }

    #[test]
    fn test_parse_cron_add_missing_schedule() {
        let err = parse_cron_add_args("bad-job --message Check something").unwrap_err();
        assert!(err.contains("missing schedule"), "{}", err);
    }

    #[test]
    fn test_parse_cron_add_empty() {
        let err = parse_cron_add_args("").unwrap_err();
        assert!(err.contains("missing job name"), "{}", err);
    }

    #[test]
    fn test_shell_split_repl_basic() {
        let tokens = shell_split_repl("hello world");
        assert_eq!(tokens, vec!["hello", "world"]);
    }

    #[test]
    fn test_shell_split_repl_quotes() {
        let tokens = shell_split_repl("--cron '0 9 * * *' --message Hello");
        assert_eq!(tokens, vec!["--cron", "0 9 * * *", "--message", "Hello"]);
    }
}
