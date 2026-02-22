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
const CMD_HEARTBEAT: &str = "/heartbeat";
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
                _ if input.starts_with(CMD_CRON) => {
                    self.handle_cron(input);
                    continue;
                }
                _ if input.starts_with(CMD_HEARTBEAT) => {
                    self.handle_heartbeat(input);
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
        let _ = writeln!(self.writer, "  /cron       Manage scheduled cron jobs");
        let _ = writeln!(self.writer, "  /heartbeat  Manage heartbeat tasks");
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
    // /heartbeat command
    // -----------------------------------------------------------------------

    fn handle_heartbeat(&mut self, input: &str) {
        let rest = input.strip_prefix(CMD_HEARTBEAT).unwrap_or("").trim();
        let parts: Vec<&str> = rest.splitn(2, char::is_whitespace).collect();
        let subcmd = parts.first().copied().unwrap_or("");
        let args_str = if parts.len() > 1 { parts[1] } else { "" };

        match subcmd {
            "show" => self.heartbeat_show(),
            "add" => self.heartbeat_add(args_str),
            "remove" => self.heartbeat_remove(args_str),
            "enable" => self.heartbeat_enable(),
            "disable" => self.heartbeat_disable(),
            "interval" => self.heartbeat_interval(args_str),
            "status" => self.heartbeat_status(),
            _ => self.heartbeat_usage(),
        }
    }

    fn heartbeat_md_path(&self) -> PathBuf {
        let config = self.load_config();
        let workspace = config.map(|c| c.workspace_path()).unwrap_or_else(|| {
            self.session
                .base_dir
                .join("workspace")
                .to_string_lossy()
                .to_string()
        });
        PathBuf::from(workspace).join("HEARTBEAT.md")
    }

    fn heartbeat_usage(&mut self) {
        let _ = writeln!(self.writer, "Usage: /heartbeat <subcommand>");
        let _ = writeln!(self.writer, "  show       Show current heartbeat tasks");
        let _ = writeln!(self.writer, "  add        Add a new task");
        let _ = writeln!(self.writer, "  remove     Remove a task by text");
        let _ = writeln!(self.writer, "  enable     Enable heartbeat in config");
        let _ = writeln!(self.writer, "  disable    Disable heartbeat in config");
        let _ = writeln!(self.writer, "  interval   Set heartbeat interval (seconds)");
        let _ = writeln!(self.writer, "  status     Show heartbeat configuration");
    }

    fn heartbeat_show(&mut self) {
        let path = self.heartbeat_md_path();
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => {
                let _ = writeln!(self.writer, "No heartbeat tasks configured");
                return;
            }
        };

        let tasks = crate::application::heartbeat::parse_heartbeat(&content);
        if tasks.is_empty() {
            let _ = writeln!(self.writer, "No heartbeat tasks configured");
            return;
        }

        let _ = writeln!(self.writer, "Heartbeat tasks:");
        for task in &tasks {
            if task.use_spawn {
                let _ = writeln!(self.writer, "  - {} [spawn]", task.message);
            } else {
                let _ = writeln!(self.writer, "  - {}", task.message);
            }
        }
        let _ = writeln!(self.writer, "{} tasks", tasks.len());
    }

    fn heartbeat_add(&mut self, args_str: &str) {
        let args_str = args_str.trim();
        let use_spawn = args_str.starts_with("--spawn");
        let task_text = if use_spawn {
            args_str.strip_prefix("--spawn").unwrap_or("").trim()
        } else {
            args_str
        };

        if task_text.is_empty() {
            let _ = writeln!(self.writer, "Error: missing task description");
            return;
        }

        let path = self.heartbeat_md_path();

        // Read existing content or start fresh
        let mut content = std::fs::read_to_string(&path).unwrap_or_default();

        if use_spawn {
            // Ensure there's a spawn section header
            if !content.to_lowercase().contains("spawn") {
                if !content.is_empty() && !content.ends_with('\n') {
                    content.push('\n');
                }
                content.push_str("## Long Tasks (use spawn)\n");
            }
            content.push_str(&format!("- {}\n", task_text));
        } else {
            // Find insertion point: before any ## header, or at end
            let insert_pos = content.find("##").unwrap_or(content.len());
            let line = format!("- {}\n", task_text);
            content.insert_str(insert_pos, &line);
        }

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        match std::fs::write(&path, &content) {
            Ok(()) => {
                let _ = writeln!(self.writer, "Task added: {}", task_text);
            }
            Err(e) => {
                let _ = writeln!(self.writer, "Error: {}", e);
            }
        }
    }

    fn heartbeat_remove(&mut self, args_str: &str) {
        let needle = args_str.trim();
        if needle.is_empty() {
            let _ = writeln!(self.writer, "Error: missing task description");
            return;
        }

        let path = self.heartbeat_md_path();
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => {
                let _ = writeln!(self.writer, "Error: task '{}' not found", needle);
                return;
            }
        };

        let mut found = false;
        let mut new_lines = Vec::new();
        for line in content.lines() {
            let trimmed = line.trim();
            if !found
                && trimmed.starts_with("- ")
                && trimmed
                    .strip_prefix("- ")
                    .is_some_and(|t| t.trim() == needle)
            {
                found = true;
                continue; // Skip this line
            }
            new_lines.push(line);
        }

        if !found {
            let _ = writeln!(self.writer, "Error: task '{}' not found", needle);
            return;
        }

        let new_content = new_lines.join("\n") + "\n";
        match std::fs::write(&path, new_content) {
            Ok(()) => {
                let _ = writeln!(self.writer, "Task removed: {}", needle);
            }
            Err(e) => {
                let _ = writeln!(self.writer, "Error: {}", e);
            }
        }
    }

    fn heartbeat_enable(&mut self) {
        self.set_heartbeat_enabled(true);
    }

    fn heartbeat_disable(&mut self) {
        self.set_heartbeat_enabled(false);
    }

    fn set_heartbeat_enabled(&mut self, enabled: bool) {
        let config_path = self.session.base_dir.join("config.json");
        match self.read_config_json(&config_path) {
            Ok(mut config) => {
                if config.get("heartbeat").is_none() {
                    config["heartbeat"] = serde_json::json!({});
                }
                config["heartbeat"]["enabled"] = serde_json::Value::Bool(enabled);
                match self.write_config_json(&config_path, &config) {
                    Ok(()) => {
                        let action = if enabled { "enabled" } else { "disabled" };
                        let _ = writeln!(self.writer, "Heartbeat {}", action);
                    }
                    Err(e) => {
                        let _ = writeln!(self.writer, "Error: {}", e);
                    }
                }
            }
            Err(e) => {
                let _ = writeln!(self.writer, "Error: {}", e);
            }
        }
    }

    fn heartbeat_interval(&mut self, args_str: &str) {
        let val_str = args_str.trim();
        let seconds: u32 = match val_str.parse() {
            Ok(0) => {
                let _ = writeln!(self.writer, "Error: interval must be at least 1 second");
                return;
            }
            Ok(n) => n,
            Err(_) => {
                let _ = writeln!(self.writer, "Error: invalid interval '{}'", val_str);
                return;
            }
        };

        let config_path = self.session.base_dir.join("config.json");
        match self.read_config_json(&config_path) {
            Ok(mut config) => {
                if config.get("heartbeat").is_none() {
                    config["heartbeat"] = serde_json::json!({});
                }
                config["heartbeat"]["interval"] =
                    serde_json::Value::Number(serde_json::Number::from(seconds));
                match self.write_config_json(&config_path, &config) {
                    Ok(()) => {
                        let _ = writeln!(self.writer, "Heartbeat interval set to {}s", seconds);
                    }
                    Err(e) => {
                        let _ = writeln!(self.writer, "Error: {}", e);
                    }
                }
            }
            Err(e) => {
                let _ = writeln!(self.writer, "Error: {}", e);
            }
        }
    }

    fn heartbeat_status(&mut self) {
        // Load config for heartbeat settings
        let config = self.load_config();
        let (hb_enabled, hb_interval) = config
            .as_ref()
            .map(|c| (c.heartbeat.enabled, c.heartbeat.interval))
            .unwrap_or((true, 30));

        let status = if hb_enabled { "enabled" } else { "disabled" };
        let _ = writeln!(self.writer, "Heartbeat: {} ({}s)", status, hb_interval);

        // Count tasks
        let path = self.heartbeat_md_path();
        let task_count = std::fs::read_to_string(&path)
            .map(|c| crate::application::heartbeat::parse_heartbeat(&c).len())
            .unwrap_or(0);
        let _ = writeln!(self.writer, "{} task(s) configured", task_count);
    }

    // -----------------------------------------------------------------------
    // /agent command
    // -----------------------------------------------------------------------

    fn handle_agent(&mut self, input: &str, rt: &tokio::runtime::Runtime) {
        let rest = input.strip_prefix(CMD_AGENT).unwrap_or("").trim();
        let parts: Vec<&str> = rest.splitn(2, char::is_whitespace).collect();
        let subcmd = parts.first().copied().unwrap_or("");
        let args_str = if parts.len() > 1 { parts[1] } else { "" };

        match subcmd {
            "list" => self.agent_list(),
            "create" => self.agent_create(args_str),
            "show" => self.agent_show(args_str),
            "edit" => self.agent_edit(args_str),
            "remove" => self.agent_remove(args_str),
            "run" => self.agent_run(args_str, rt),
            _ => self.agent_usage(),
        }
    }

    fn agents_dir(&self) -> PathBuf {
        self.session.base_dir.join("agents")
    }

    /// Validate and return the path to an agent profile. Returns an error
    /// message if the name is empty or contains path traversal characters.
    fn validated_agent_path(&mut self, name: &str) -> Option<PathBuf> {
        if name.is_empty() {
            let _ = writeln!(self.writer, "Error: missing agent name");
            return None;
        }
        if !is_valid_agent_name(name) {
            let _ = writeln!(self.writer, "Error: invalid agent name '{}'", name);
            return None;
        }
        Some(self.agents_dir().join(format!("{}.json", name)))
    }

    fn agent_usage(&mut self) {
        let _ = writeln!(self.writer, "Usage: /agent <subcommand>");
        let _ = writeln!(self.writer, "  list     List all subagent profiles");
        let _ = writeln!(self.writer, "  create   Create a new profile");
        let _ = writeln!(self.writer, "  show     Show a profile's configuration");
        let _ = writeln!(self.writer, "  edit     Edit an existing profile");
        let _ = writeln!(self.writer, "  remove   Remove a profile");
        let _ = writeln!(self.writer, "  run      Run a task using a profile");
    }

    fn agent_list(&mut self) {
        let dir = self.agents_dir();
        if !dir.exists() {
            let _ = writeln!(self.writer, "No subagent profiles configured");
            return;
        }

        let entries: Vec<String> = std::fs::read_dir(&dir)
            .ok()
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
                    .filter_map(|e| {
                        e.path()
                            .file_stem()
                            .map(|s| s.to_string_lossy().to_string())
                    })
                    .collect()
            })
            .unwrap_or_default();

        if entries.is_empty() {
            let _ = writeln!(self.writer, "No subagent profiles configured");
            return;
        }

        let _ = writeln!(self.writer, "Subagent profiles:");
        for name in &entries {
            let _ = writeln!(self.writer, "  {}", name);
        }
    }

    fn agent_create(&mut self, args_str: &str) {
        match parse_agent_args(args_str) {
            Ok(parsed) => {
                if parsed.system.is_none() {
                    let _ = writeln!(self.writer, "Error: missing required flag: --system");
                    return;
                }

                // Validate name
                if !is_valid_agent_name(&parsed.name) {
                    let _ = writeln!(self.writer, "Error: invalid agent name '{}'", parsed.name);
                    return;
                }

                let dir = self.agents_dir();
                let path = dir.join(format!("{}.json", parsed.name));

                if path.exists() {
                    let _ = writeln!(self.writer, "Error: agent '{}' already exists", parsed.name);
                    return;
                }

                if let Err(e) = std::fs::create_dir_all(&dir) {
                    let _ = writeln!(self.writer, "Error: {}", e);
                    return;
                }

                let mut profile = serde_json::json!({
                    "name": parsed.name,
                    "system": parsed.system
                });
                if let Some(ref model) = parsed.model {
                    profile["model"] = serde_json::json!(model);
                }

                match serde_json::to_string_pretty(&profile) {
                    Ok(content) => match std::fs::write(&path, content) {
                        Ok(()) => {
                            let _ = writeln!(self.writer, "Agent '{}' created", parsed.name);
                        }
                        Err(e) => {
                            let _ = writeln!(self.writer, "Error: {}", e);
                        }
                    },
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

    fn agent_show(&mut self, args_str: &str) {
        let name = args_str.trim();
        let Some(path) = self.validated_agent_path(name) else {
            return;
        };
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                if let Ok(profile) = serde_json::from_str::<serde_json::Value>(&content) {
                    let _ = writeln!(self.writer, "Agent: {}", name);
                    if let Some(system) = profile["system"].as_str() {
                        let _ = writeln!(self.writer, "System: {}", system);
                    }
                    if let Some(model) = profile["model"].as_str() {
                        let _ = writeln!(self.writer, "Model: {}", model);
                    }
                } else {
                    let _ = writeln!(self.writer, "Error: invalid profile for '{}'", name);
                }
            }
            Err(_) => {
                let _ = writeln!(self.writer, "Error: agent '{}' not found", name);
            }
        }
    }

    fn agent_edit(&mut self, args_str: &str) {
        match parse_agent_args(args_str) {
            Ok(parsed) => {
                let Some(path) = self.validated_agent_path(&parsed.name) else {
                    return;
                };
                match std::fs::read_to_string(&path) {
                    Ok(content) => {
                        let mut profile: serde_json::Value =
                            serde_json::from_str(&content).unwrap_or_default();
                        if let Some(ref system) = parsed.system {
                            profile["system"] = serde_json::json!(system);
                        }
                        if let Some(ref model) = parsed.model {
                            profile["model"] = serde_json::json!(model);
                        }
                        match serde_json::to_string_pretty(&profile) {
                            Ok(updated) => match std::fs::write(&path, updated) {
                                Ok(()) => {
                                    let _ =
                                        writeln!(self.writer, "Agent '{}' updated", parsed.name);
                                }
                                Err(e) => {
                                    let _ = writeln!(self.writer, "Error: {}", e);
                                }
                            },
                            Err(e) => {
                                let _ = writeln!(self.writer, "Error: {}", e);
                            }
                        }
                    }
                    Err(_) => {
                        let _ = writeln!(self.writer, "Error: agent '{}' not found", parsed.name);
                    }
                }
            }
            Err(msg) => {
                let _ = writeln!(self.writer, "Error: {}", msg);
            }
        }
    }

    fn agent_remove(&mut self, args_str: &str) {
        let name = args_str.trim();
        let Some(path) = self.validated_agent_path(name) else {
            return;
        };
        if !path.exists() {
            let _ = writeln!(self.writer, "Error: agent '{}' not found", name);
            return;
        }

        match std::fs::remove_file(&path) {
            Ok(()) => {
                let _ = writeln!(self.writer, "Agent '{}' removed", name);
            }
            Err(e) => {
                let _ = writeln!(self.writer, "Error: {}", e);
            }
        }
    }

    fn agent_run(&mut self, args_str: &str, rt: &tokio::runtime::Runtime) {
        let parts: Vec<&str> = args_str.splitn(2, char::is_whitespace).collect();
        let name = parts.first().copied().unwrap_or("").trim();
        let task = if parts.len() > 1 { parts[1].trim() } else { "" };

        let Some(path) = self.validated_agent_path(name) else {
            return;
        };

        // Load profile
        let profile = match std::fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
                Ok(p) => p,
                Err(_) => {
                    let _ = writeln!(self.writer, "Error: invalid profile for '{}'", name);
                    return;
                }
            },
            Err(_) => {
                let _ = writeln!(self.writer, "Error: agent '{}' not found", name);
                return;
            }
        };

        if task.is_empty() {
            let _ = writeln!(self.writer, "Error: missing task description");
            return;
        }

        // Get the profile's system prompt
        let system = profile["system"].as_str().unwrap_or("").to_string();

        // Build ephemeral message list (same pattern as /spawn for session isolation)
        let mut run_messages = Vec::new();

        if !system.is_empty() {
            run_messages.push(Message {
                role: Role::System,
                content: system,
                tool_calls: vec![],
                tool_call_id: None,
            });
        }

        run_messages.push(Message {
            role: Role::User,
            content: task.to_string(),
            tool_calls: vec![],
            tool_call_id: None,
        });

        let result = rt.block_on(self.session.agent.process(&mut run_messages));

        match result {
            Ok(r) => {
                // Inject only the result into the parent conversation
                self.session.messages.push(Message {
                    role: Role::Assistant,
                    content: r.response.clone(),
                    tool_calls: vec![],
                    tool_call_id: None,
                });
                let _ = writeln!(self.writer, "{}", r.response);
            }
            Err(e) => {
                let _ = writeln!(self.writer, "Error: {e}");
            }
        }
    }

    // -----------------------------------------------------------------------
    // /spawn command
    // -----------------------------------------------------------------------

    fn handle_spawn(&mut self, input: &str, rt: &tokio::runtime::Runtime) {
        let rest = input.strip_prefix(CMD_SPAWN).unwrap_or("").trim();

        if rest.is_empty() {
            let _ = writeln!(self.writer, "Error: missing task description");
            return;
        }

        // Parse flags from the arguments
        let parsed = match parse_spawn_args(rest) {
            Ok(p) => p,
            Err(msg) => {
                let _ = writeln!(self.writer, "Error: {}", msg);
                return;
            }
        };

        if parsed.help {
            self.spawn_usage();
            return;
        }

        if parsed.task.is_empty() {
            let _ = writeln!(self.writer, "Error: missing task description");
            return;
        }

        if parsed.model.is_some() {
            let _ = writeln!(
                self.writer,
                "Error: --model is not supported in REPL mode (agent uses the model from startup)"
            );
            return;
        }

        // Resolve system prompt: from --agent profile or --system flag
        let system = if let Some(ref agent_name) = parsed.agent {
            let Some(path) = self.validated_agent_path(agent_name) else {
                return;
            };
            match std::fs::read_to_string(&path) {
                Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
                    Ok(profile) => profile["system"].as_str().map(String::from),
                    Err(_) => {
                        let _ =
                            writeln!(self.writer, "Error: invalid profile for '{}'", agent_name);
                        return;
                    }
                },
                Err(_) => {
                    let _ = writeln!(self.writer, "Error: agent '{}' not found", agent_name);
                    return;
                }
            }
        } else {
            parsed.system.clone()
        };

        // Build an ephemeral message list for the spawn (session isolation).
        // The spawn task and system prompt are NOT added to the parent session.
        let mut spawn_messages = Vec::new();

        if let Some(ref prompt) = system {
            spawn_messages.push(Message {
                role: Role::System,
                content: prompt.clone(),
                tool_calls: vec![],
                tool_call_id: None,
            });
        }

        spawn_messages.push(Message {
            role: Role::User,
            content: parsed.task.clone(),
            tool_calls: vec![],
            tool_call_id: None,
        });

        // Run with optional timeout
        let result = if let Some(max_secs) = parsed.max_time {
            let timeout = std::time::Duration::from_secs(max_secs);
            rt.block_on(async {
                match tokio::time::timeout(timeout, self.session.agent.process(&mut spawn_messages))
                    .await
                {
                    Ok(r) => r,
                    Err(_) => Err(crate::domain::error::DomainError::Other(
                        "spawn timed out".to_string(),
                    )),
                }
            })
        } else {
            rt.block_on(self.session.agent.process(&mut spawn_messages))
        };

        match result {
            Ok(r) => {
                // Inject only the result into the parent conversation
                // so the LLM can reference it in subsequent turns.
                self.session.messages.push(Message {
                    role: Role::Assistant,
                    content: r.response.clone(),
                    tool_calls: vec![],
                    tool_call_id: None,
                });
                let _ = writeln!(self.writer, "{}", r.response);
            }
            Err(e) => {
                let _ = writeln!(self.writer, "Error: {e}");
            }
        }
    }

    fn spawn_usage(&mut self) {
        let _ = writeln!(self.writer, "Usage: /spawn [flags] <task>");
        let _ = writeln!(
            self.writer,
            "  --agent <name>       Use a named agent profile"
        );
        let _ = writeln!(self.writer, "  --system <prompt>    Set a system prompt");
        let _ = writeln!(
            self.writer,
            "  --model <model>      Not supported in REPL mode"
        );
        let _ = writeln!(
            self.writer,
            "  --max-time <secs>    Set a timeout in seconds"
        );
        let _ = writeln!(self.writer, "  --help               Show this help");
    }

    // -----------------------------------------------------------------------
    // Config helpers
    // -----------------------------------------------------------------------

    fn load_config(&self) -> Option<Config> {
        let config_path = self.session.base_dir.join("config.json");
        Config::load(config_path.to_str()?).ok()
    }

    fn read_config_json(&self, path: &Path) -> Result<serde_json::Value, String> {
        let content = std::fs::read_to_string(path).map_err(|e| format!("read config: {}", e))?;
        serde_json::from_str(&content).map_err(|e| format!("parse config: {}", e))
    }

    fn write_config_json(&self, path: &Path, config: &serde_json::Value) -> Result<(), String> {
        let content =
            serde_json::to_string_pretty(config).map_err(|e| format!("serialize config: {}", e))?;
        std::fs::write(path, content).map_err(|e| format!("write config: {}", e))
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
/// Uses `chars()` iteration to correctly handle multi-byte UTF-8.
fn shell_split_repl(s: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut chars = s.chars().peekable();

    while let Some(&ch) = chars.peek() {
        if ch == ' ' {
            chars.next();
            continue;
        }
        let mut current = String::new();
        if ch == '\'' || ch == '"' {
            let quote = ch;
            chars.next();
            while let Some(&c) = chars.peek() {
                if c == quote {
                    chars.next();
                    break;
                }
                current.push(c);
                chars.next();
            }
        } else {
            while let Some(&c) = chars.peek() {
                if c == ' ' || c == '\'' || c == '"' {
                    break;
                }
                current.push(c);
                chars.next();
            }
        }
        if !current.is_empty() {
            tokens.push(current);
        }
    }
    tokens
}

// ===========================================================================
// /agent argument parser
// ===========================================================================

/// Parsed arguments for `/agent create` or `/agent edit`.
#[derive(Debug)]
struct ParsedAgentArgs {
    name: String,
    system: Option<String>,
    model: Option<String>,
}

/// Parse `/agent create|edit <name> [--system ...] [--model ...]`
///
/// The name is the first token. `--system` collects all subsequent tokens
/// until the next `--` flag (or end). `--model` takes a single token.
fn parse_agent_args(args_str: &str) -> Result<ParsedAgentArgs, String> {
    let tokens = shell_split_repl(args_str);
    if tokens.is_empty() {
        return Err("missing agent name".to_string());
    }

    let name = tokens[0].clone();
    let mut system: Option<String> = None;
    let mut model: Option<String> = None;

    let mut i = 1;
    while i < tokens.len() {
        match tokens[i].as_str() {
            "--system" => {
                if i + 1 < tokens.len() {
                    let mut parts = Vec::new();
                    i += 1;
                    while i < tokens.len() && !tokens[i].starts_with("--") {
                        parts.push(tokens[i].clone());
                        i += 1;
                    }
                    if parts.is_empty() {
                        return Err("--system requires a value".to_string());
                    }
                    system = Some(parts.join(" "));
                } else {
                    return Err("--system requires a value".to_string());
                }
            }
            "--model" => {
                if i + 1 < tokens.len() {
                    model = Some(tokens[i + 1].clone());
                    i += 2;
                } else {
                    return Err("--model requires a value".to_string());
                }
            }
            _ => {
                i += 1;
            }
        }
    }

    Ok(ParsedAgentArgs {
        name,
        system,
        model,
    })
}

/// Check whether an agent name is valid (safe for use as a filename).
///
/// Allowed: ASCII alphanumeric, hyphens, underscores. 1-64 characters.
fn is_valid_agent_name(name: &str) -> bool {
    let len = name.len();
    if len == 0 || len > 64 {
        return false;
    }
    name.chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

// ===========================================================================
// /spawn argument parser
// ===========================================================================

/// Parsed arguments for `/spawn`.
#[derive(Debug)]
struct ParsedSpawnArgs {
    agent: Option<String>,
    system: Option<String>,
    model: Option<String>,
    max_time: Option<u64>,
    task: String,
    help: bool,
}

/// Parse `/spawn [--agent name] [--system prompt] [--model model] [--max-time secs] [--help] <task>`
fn parse_spawn_args(args_str: &str) -> Result<ParsedSpawnArgs, String> {
    let tokens = shell_split_repl(args_str);
    if tokens.is_empty() {
        return Ok(ParsedSpawnArgs {
            agent: None,
            system: None,
            model: None,
            max_time: None,
            task: String::new(),
            help: false,
        });
    }

    let mut agent: Option<String> = None;
    let mut system: Option<String> = None;
    let mut model: Option<String> = None;
    let mut max_time: Option<u64> = None;
    let mut help = false;
    let mut task_parts = Vec::new();

    let mut i = 0;
    while i < tokens.len() {
        match tokens[i].as_str() {
            "--help" => {
                help = true;
                i += 1;
            }
            "--agent" => {
                if i + 1 < tokens.len() {
                    agent = Some(tokens[i + 1].clone());
                    i += 2;
                } else {
                    return Err("--agent requires a value".to_string());
                }
            }
            "--system" => {
                if i + 1 < tokens.len() {
                    // Take the next token as the system prompt. If the user
                    // wants a multi-word prompt they must quote it:
                    //   /spawn --system 'You are a translator' task
                    system = Some(tokens[i + 1].clone());
                    i += 2;
                } else {
                    return Err("--system requires a value".to_string());
                }
            }
            "--model" => {
                if i + 1 < tokens.len() {
                    model = Some(tokens[i + 1].clone());
                    i += 2;
                } else {
                    return Err("--model requires a value".to_string());
                }
            }
            "--max-time" => {
                if i + 1 < tokens.len() {
                    max_time = Some(
                        tokens[i + 1]
                            .parse::<u64>()
                            .map_err(|_| "invalid --max-time value".to_string())?,
                    );
                    i += 2;
                } else {
                    return Err("--max-time requires a value".to_string());
                }
            }
            _ => {
                // Everything else is the task
                task_parts.push(tokens[i].clone());
                i += 1;
            }
        }
    }

    Ok(ParsedSpawnArgs {
        agent,
        system,
        model,
        max_time,
        task: task_parts.join(" "),
        help,
    })
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
        assert_eq!(CMD_HEARTBEAT, "/heartbeat");
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

    #[test]
    fn test_shell_split_repl_utf8() {
        // Multi-byte UTF-8: accented chars, CJK, emoji
        let tokens = shell_split_repl("--message 'café résumé'");
        assert_eq!(tokens, vec!["--message", "café résumé"]);

        let tokens = shell_split_repl("hello 世界");
        assert_eq!(tokens, vec!["hello", "世界"]);

        let tokens = shell_split_repl("--system '你好世界' task");
        assert_eq!(tokens, vec!["--system", "你好世界", "task"]);
    }

    // -- Heartbeat add argument parsing tests --

    #[test]
    fn test_heartbeat_add_spawn_flag_parsing() {
        // --spawn flag should be stripped and remainder used as task text
        let input = "--spawn Analyze monthly data";
        let use_spawn = input.starts_with("--spawn");
        let task_text = input.strip_prefix("--spawn").unwrap_or("").trim();
        assert!(use_spawn);
        assert_eq!(task_text, "Analyze monthly data");
    }

    #[test]
    fn test_heartbeat_add_no_spawn_flag() {
        let input = "Regular task";
        let use_spawn = input.starts_with("--spawn");
        let task_text = if use_spawn {
            input.strip_prefix("--spawn").unwrap_or("").trim()
        } else {
            input
        };
        assert!(!use_spawn);
        assert_eq!(task_text, "Regular task");
    }

    #[test]
    fn test_heartbeat_add_empty_after_spawn() {
        let input = "--spawn";
        let use_spawn = input.starts_with("--spawn");
        let task_text = input.strip_prefix("--spawn").unwrap_or("").trim();
        assert!(use_spawn);
        assert!(task_text.is_empty());
    }

    #[test]
    fn test_heartbeat_interval_parsing_valid() {
        let val: Result<u32, _> = "300".parse();
        assert_eq!(val.unwrap(), 300);
    }

    #[test]
    fn test_heartbeat_interval_parsing_invalid() {
        let val: Result<u32, _> = "abc".parse();
        assert!(val.is_err());
    }

    // -- /agent argument parser tests --

    #[test]
    fn test_parse_agent_args_with_system() {
        let parsed = parse_agent_args("researcher --system You are a research specialist").unwrap();
        assert_eq!(parsed.name, "researcher");
        assert_eq!(
            parsed.system.as_deref(),
            Some("You are a research specialist")
        );
        assert!(parsed.model.is_none());
    }

    #[test]
    fn test_parse_agent_args_with_system_and_model() {
        let parsed =
            parse_agent_args("fast-bot --system Quick answers only --model gpt-5-mini").unwrap();
        assert_eq!(parsed.name, "fast-bot");
        assert_eq!(parsed.system.as_deref(), Some("Quick answers only"));
        assert_eq!(parsed.model.as_deref(), Some("gpt-5-mini"));
    }

    #[test]
    fn test_parse_agent_args_model_only() {
        let parsed = parse_agent_args("researcher --model gpt-5-mini").unwrap();
        assert_eq!(parsed.name, "researcher");
        assert!(parsed.system.is_none());
        assert_eq!(parsed.model.as_deref(), Some("gpt-5-mini"));
    }

    #[test]
    fn test_parse_agent_args_empty() {
        let err = parse_agent_args("").unwrap_err();
        assert!(err.contains("missing agent name"), "{}", err);
    }

    #[test]
    fn test_parse_agent_args_name_only() {
        let parsed = parse_agent_args("nameless").unwrap();
        assert_eq!(parsed.name, "nameless");
        assert!(parsed.system.is_none());
        assert!(parsed.model.is_none());
    }

    // -- Agent name validation tests --

    #[test]
    fn test_valid_agent_names() {
        assert!(is_valid_agent_name("researcher"));
        assert!(is_valid_agent_name("fast-bot"));
        assert!(is_valid_agent_name("my_agent_1"));
        assert!(is_valid_agent_name("A"));
    }

    #[test]
    fn test_invalid_agent_names() {
        assert!(!is_valid_agent_name(""));
        assert!(!is_valid_agent_name("../escape"));
        assert!(!is_valid_agent_name("bad name"));
        assert!(!is_valid_agent_name("bad/name"));
        assert!(!is_valid_agent_name("bad.name"));
        assert!(!is_valid_agent_name(&"a".repeat(65)));
    }

    // -- /spawn argument parser tests --

    #[test]
    fn test_parse_spawn_args_simple_task() {
        let parsed = parse_spawn_args("What is the meaning of life?").unwrap();
        assert_eq!(parsed.task, "What is the meaning of life?");
        assert!(parsed.agent.is_none());
        assert!(parsed.system.is_none());
        assert!(parsed.max_time.is_none());
        assert!(!parsed.help);
    }

    #[test]
    fn test_parse_spawn_args_with_agent() {
        let parsed =
            parse_spawn_args("--agent researcher What is new in quantum computing?").unwrap();
        assert_eq!(parsed.agent.as_deref(), Some("researcher"));
        assert_eq!(parsed.task, "What is new in quantum computing?");
    }

    #[test]
    fn test_parse_spawn_args_with_system() {
        let parsed = parse_spawn_args("--system 'You are a translator' Translate: hello").unwrap();
        assert_eq!(parsed.system.as_deref(), Some("You are a translator"));
        assert_eq!(parsed.task, "Translate: hello");
    }

    #[test]
    fn test_parse_spawn_args_with_model() {
        let parsed = parse_spawn_args("--model gpt-5-mini Summarize briefly").unwrap();
        assert_eq!(parsed.model.as_deref(), Some("gpt-5-mini"));
        assert_eq!(parsed.task, "Summarize briefly");
    }

    #[test]
    fn test_parse_spawn_args_with_max_time() {
        let parsed = parse_spawn_args("--max-time 30 Slow task").unwrap();
        assert_eq!(parsed.max_time, Some(30));
        assert_eq!(parsed.task, "Slow task");
    }

    #[test]
    fn test_parse_spawn_args_help() {
        let parsed = parse_spawn_args("--help").unwrap();
        assert!(parsed.help);
    }

    #[test]
    fn test_parse_spawn_args_empty() {
        let parsed = parse_spawn_args("").unwrap();
        assert!(parsed.task.is_empty());
    }

    #[test]
    fn test_parse_spawn_args_combined_flags() {
        let parsed =
            parse_spawn_args("--agent bot --system 'Custom prompt' --max-time 60 Do the thing")
                .unwrap();
        assert_eq!(parsed.agent.as_deref(), Some("bot"));
        assert_eq!(parsed.system.as_deref(), Some("Custom prompt"));
        assert_eq!(parsed.max_time, Some(60));
        assert_eq!(parsed.task, "Do the thing");
    }
}
