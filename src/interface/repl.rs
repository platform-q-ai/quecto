use std::io::{BufRead, Write};
use std::path::Path;
use std::sync::Arc;

use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::domain::agent::AgentLoop;
use crate::domain::message::{Message, Role};
use crate::domain::provider::LlmProvider;
use crate::domain::session::{Session, SessionStore};
use crate::infrastructure::config::Config;
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
        let _ = writeln!(self.writer, "  /help   Show this help");
        let _ = writeln!(self.writer, "  /clear  Clear conversation history");
        let _ = writeln!(self.writer, "  /exit   Exit the REPL");
        let _ = writeln!(self.writer, "  /quit   Exit the REPL");
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
}
