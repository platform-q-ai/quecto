#![allow(private_interfaces)]

use cucumber::{World, gherkin, given, then, when};
use quecto::application::agent_loop::AgentLoopImpl;
use quecto::application::cron_executor;
use quecto::application::heartbeat::{self, HeartbeatResult, HeartbeatTask, HeartbeatTaskResult};
use quecto::application::subagent::{SubagentConfig, SubagentContext, validate_agent_id};
use quecto::application::voice as app_voice;
use quecto::domain::agent::{AgentInfo, AgentLoop, AgentResult};
use quecto::domain::cron::{CronJob, CronJobResult, CronSchedule, CronStore};
use quecto::domain::error::DomainError;
use quecto::domain::message::{LlmResponse, Message, Role, ToolCall};
use quecto::domain::provider::LlmProvider;
use quecto::domain::session::{Session, SessionStore};
use quecto::domain::skill::{Skill, SkillLoader};
use quecto::domain::tool::{Tool, ToolDefinition, ToolResult};
use quecto::infrastructure::auth::credential_store::{
    AuthMethod, Credential, CredentialStatus, CredentialStore,
};
use quecto::infrastructure::bus::{MessageBus, OutboundMessage};
use quecto::infrastructure::channels::telegram::{
    TelegramChannel, TelegramChat, TelegramMessage, TelegramUpdate, TelegramUpdateMessage,
    TelegramUser,
};
use quecto::infrastructure::config::{Config, TelegramConfig};
use quecto::infrastructure::health::server::StaticReadiness;

use quecto::infrastructure::persistence::cron_store::FileCronStore;
use quecto::infrastructure::persistence::memory_store::{self, MemoryStore};
use quecto::infrastructure::persistence::session_store::FileSessionStore;
use quecto::infrastructure::persistence::skill_loader::FileSkillLoader;
use quecto::infrastructure::providers;
use quecto::infrastructure::providers::error::ErrorClass;
use quecto::infrastructure::providers::fallback::FallbackProvider;
use quecto::infrastructure::security::sandbox::Sandbox;
use quecto::infrastructure::tools::exec::ExecTool;
use quecto::infrastructure::tools::message::MessageTool;
use quecto::infrastructure::tools::registry::ToolRegistryImpl;
use quecto::infrastructure::tools::spawn::SpawnTool;
use quecto::infrastructure::tools::web_search::WebSearchTool;
use quecto::infrastructure::voice::groq_whisper::{GroqWhisperClient, TranscriptionResult};
use quecto::interface::cli::{self, CliContext};
use quecto::interface::gateway::handle_bot_command;
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

// ===========================================================================
// Mock LLM Provider for BDD tests
// ===========================================================================

#[derive(Debug)]
struct MockLlmProvider {
    /// Queue of responses to return (FIFO).
    responses: Mutex<Vec<LlmResponse>>,
    /// Captured tool definitions from the most recent chat() call.
    last_tool_defs: Mutex<Vec<ToolDefinition>>,
}

impl MockLlmProvider {
    fn new() -> Self {
        Self {
            responses: Mutex::new(vec![]),
            last_tool_defs: Mutex::new(vec![]),
        }
    }

    fn push_response(&self, response: LlmResponse) {
        self.responses.lock().unwrap().push(response);
    }

    fn last_tool_defs(&self) -> Vec<ToolDefinition> {
        self.last_tool_defs.lock().unwrap().clone()
    }
}

impl LlmProvider for MockLlmProvider {
    fn name(&self) -> &str {
        "mock"
    }

    fn chat(
        &self,
        request: quecto::domain::provider::ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        *self.last_tool_defs.lock().unwrap() = request.tools.to_vec();
        let response = {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                LlmResponse {
                    content: Some("(no more responses)".to_string()),
                    tool_calls: vec![],
                    usage: None,
                }
            } else {
                responses.remove(0)
            }
        };
        Box::pin(async move { Ok(response) })
    }
}

// ===========================================================================
// Mock Tool for BDD agent_loop tests
// ===========================================================================

struct MockBddTool {
    def: ToolDefinition,
    response: Mutex<String>,
}

impl MockBddTool {
    fn new(name: &str, response: &str) -> Self {
        Self {
            def: ToolDefinition {
                name: name.to_string(),
                description: format!("Mock {} tool", name),
                parameters_schema: r#"{"type":"object","properties":{}}"#.to_string(),
            },
            response: Mutex::new(response.to_string()),
        }
    }

    #[allow(dead_code)]
    fn set_response(&self, response: &str) {
        *self.response.lock().unwrap() = response.to_string();
    }
}

impl std::fmt::Debug for MockBddTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockBddTool")
            .field("name", &self.def.name)
            .finish()
    }
}

impl quecto::domain::tool::Tool for MockBddTool {
    fn definition(&self) -> ToolDefinition {
        self.def.clone()
    }

    fn execute(
        &self,
        _arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let content = self.response.lock().unwrap().clone();
        Box::pin(async move {
            Ok(ToolResult {
                content,
                is_error: false,
            })
        })
    }
}

// ===========================================================================
// In-memory CronStore for gateway BDD tests
// ===========================================================================

#[derive(Debug)]
struct InMemoryCronStore {
    jobs: Mutex<Vec<CronJob>>,
}

impl InMemoryCronStore {
    fn new() -> Self {
        Self {
            jobs: Mutex::new(vec![]),
        }
    }
}

impl CronStore for InMemoryCronStore {
    fn list(&self) -> Result<Vec<CronJob>, DomainError> {
        Ok(self.jobs.lock().unwrap().clone())
    }
    fn add(&self, job: CronJob) -> Result<(), DomainError> {
        self.jobs.lock().unwrap().push(job);
        Ok(())
    }
    fn remove(&self, id: &str) -> Result<(), DomainError> {
        self.jobs.lock().unwrap().retain(|j| j.id != id);
        Ok(())
    }
    fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), DomainError> {
        if let Some(j) = self.jobs.lock().unwrap().iter_mut().find(|j| j.id == id) {
            j.enabled = enabled;
        }
        Ok(())
    }
    fn find_by_name(&self, name: &str) -> Result<Option<CronJob>, DomainError> {
        Ok(self
            .jobs
            .lock()
            .unwrap()
            .iter()
            .find(|j| j.name == name)
            .cloned())
    }
    fn set_last_error(&self, id: &str, error: Option<String>) -> Result<(), DomainError> {
        if let Some(j) = self.jobs.lock().unwrap().iter_mut().find(|j| j.id == id) {
            j.last_error = error;
        }
        Ok(())
    }
    fn set_last_run_at(&self, id: &str, timestamp: u64) -> Result<(), DomainError> {
        if let Some(j) = self.jobs.lock().unwrap().iter_mut().find(|j| j.id == id) {
            j.last_run_at = timestamp;
        }
        Ok(())
    }
}

// ===========================================================================
// Recording mock agent for gateway BDD tests
// ===========================================================================

#[derive(Debug)]
struct RecordingMockAgent {
    response: String,
    messages: Arc<Mutex<Vec<String>>>,
}

impl AgentLoop for RecordingMockAgent {
    fn process<'a>(
        &'a self,
        messages: &'a mut Vec<Message>,
    ) -> Pin<Box<dyn Future<Output = Result<AgentResult, DomainError>> + Send + 'a>> {
        let user_msg = messages
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .map(|m| m.content.clone())
            .unwrap_or_default();
        self.messages.lock().unwrap().push(user_msg);
        let resp = self.response.clone();
        Box::pin(async move { Ok(AgentResult::text(resp)) })
    }

    fn info(&self) -> AgentInfo {
        AgentInfo {
            tool_count: 0,
            skill_count: 0,
        }
    }
}

// ===========================================================================
// Slow mock agent (for timeout tests)
// ===========================================================================

#[derive(Debug)]
struct SlowMockAgent;

impl AgentLoop for SlowMockAgent {
    fn process<'a>(
        &'a self,
        _messages: &'a mut Vec<Message>,
    ) -> Pin<Box<dyn Future<Output = Result<AgentResult, DomainError>> + Send + 'a>> {
        Box::pin(async {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            Ok(AgentResult::text("done"))
        })
    }

    fn info(&self) -> AgentInfo {
        AgentInfo {
            tool_count: 0,
            skill_count: 0,
        }
    }
}

// Wrapper for Arc<dyn AgentLoop> that implements Debug (opaque).
struct DebugAgent(Arc<dyn AgentLoop>);

impl std::fmt::Debug for DebugAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<MockAgent>")
    }
}

impl Default for DebugAgent {
    fn default() -> Self {
        Self(Arc::new(RecordingMockAgent {
            response: String::new(),
            messages: Arc::new(Mutex::new(vec![])),
        }))
    }
}

#[derive(Debug, Default, World)]
pub struct QuectoWorld {
    /// Exit code from the last CLI invocation
    pub exit_code: i32,
    /// Captured stdout from the last CLI invocation
    pub stdout: String,
    /// Captured stderr from the last CLI invocation
    pub stderr: String,
    /// Path to a temporary config file used in tests
    pub config_path: Option<String>,
    /// Path to a temporary workspace directory used in tests
    pub workspace_path: Option<String>,
    /// Loaded config (after "When I load the config")
    pub config: Option<Config>,
    /// Resolved workspace path (after "When I resolve the workspace path")
    pub resolved_workspace: Option<String>,
    /// Environment variable overrides to apply during config loading
    pub env_overrides: HashMap<String, String>,
    /// CLI context (allows overriding base_dir for onboard etc.)
    pub cli_context: CliContext,
    /// Security sandbox for testing path/command validation
    pub sandbox: Option<Sandbox>,
    /// Result of the last sandbox validation (Ok or Err message)
    pub validation_result: Option<Result<(), String>>,
    /// Tool registry for agent_tools scenarios
    pub tool_registry: Option<ToolRegistryImpl>,
    /// Path to the tool workspace (for file assertions)
    pub tool_workspace: Option<PathBuf>,
    /// Result of the last tool execution
    pub tool_result: Option<Result<ToolResult, String>>,
    /// Created LLM provider
    pub provider: Option<Arc<dyn LlmProvider>>,
    /// Error classification result
    pub error_class: Option<ErrorClass>,
    /// Fallback provider for fallback/cooldown scenarios
    pub fallback_provider: Option<Arc<FallbackProvider>>,
    /// Response from fallback provider
    pub fallback_response: Option<LlmResponse>,
    /// Mock LLM provider for agent_loop scenarios
    pub mock_llm: Option<Arc<MockLlmProvider>>,
    /// Agent loop result from the last process() call
    pub agent_result: Option<AgentResult>,
    /// Agent info from the last info() call
    pub agent_info: Option<AgentInfo>,
    /// Mock tools registered for agent_loop scenarios (for inspection)
    pub mock_tools: HashMap<String, Arc<MockBddTool>>,
    /// Tool execution order tracking
    pub executed_tools: Arc<Mutex<Vec<String>>>,
    /// Session workspace path (for session scenarios)
    pub session_workspace: Option<PathBuf>,
    /// Session store for session scenarios
    pub session_store: Option<FileSessionStore>,
    /// Loaded session (after a load operation)
    pub loaded_session: Option<Option<Session>>,
    /// Memory store for memory scenarios
    pub memory_store: Option<MemoryStore>,
    /// Loaded identity content
    pub loaded_identity: Option<String>,
    /// Session keys created during routing scenarios
    pub session_keys: HashMap<String, String>,
    /// Credential store for auth scenarios
    pub credential_store: Option<CredentialStore>,
    /// Auth status summary from the last check
    pub auth_status: Option<Vec<CredentialStatus>>,
    /// Cron store for cron scenarios
    pub cron_store: Option<FileCronStore>,
    /// Cron workspace path
    pub cron_workspace: Option<PathBuf>,
    /// Listed cron jobs
    pub cron_jobs: Option<Vec<CronJob>>,
    /// Telegram config for deferred channel creation
    pub telegram_config: Option<TelegramConfig>,
    /// Result of checking whether Telegram is enabled (without creating a channel)
    pub telegram_enabled_check: Option<bool>,
    /// Telegram channel for telegram scenarios
    pub telegram_channel: Option<TelegramChannel>,
    /// Whether the last message passed the allow_from filter
    pub telegram_filter_result: Option<bool>,
    /// Parsed Telegram message from update parsing
    pub telegram_parsed_message: Option<TelegramMessage>,
    /// Raw Telegram update for parsing scenarios
    pub telegram_update: Option<TelegramUpdate>,
    /// Message bus for message tool scenarios
    pub message_bus_receiver: Option<tokio::sync::mpsc::Receiver<OutboundMessage>>,
    /// Spawn tool result
    pub spawn_result: Option<ToolResult>,
    /// Spawn tool for BDD
    pub spawn_tool: Option<SpawnTool>,
    /// Skill loader workspace directory for skills scenarios
    pub skill_loader_workspace: Option<PathBuf>,
    /// Listed skills from skill loader
    pub skill_list: Option<Vec<Skill>>,
    /// Loaded single skill
    pub loaded_skill: Option<Option<Skill>>,
    /// Temp dirs for skill tests (keep alive)
    pub _skill_temp_dirs: Vec<TempDir>,
    /// Raw heartbeat content for parsing
    pub heartbeat_content: Option<String>,
    /// Parsed heartbeat tasks
    pub heartbeat_tasks: Option<Vec<HeartbeatTask>>,
    /// Heartbeat workspace path
    pub heartbeat_workspace: Option<PathBuf>,
    /// Heartbeat result for status scenarios
    pub heartbeat_result: Option<HeartbeatResult>,
    /// Subagent spawn config for subagent scenarios
    pub subagent_config: Option<SubagentConfig>,
    /// Created subagent context
    pub subagent_context: Option<SubagentContext>,
    /// Agent allowlist for subagent validation scenarios
    pub agent_allowlist: Vec<String>,
    /// Result of agent_id validation
    pub agent_id_validation: Option<Result<(), String>>,
    /// Groq Whisper client for voice scenarios
    pub whisper_client: Option<GroqWhisperClient>,
    /// Wiremock server for voice scenarios (kept alive via Box leak)
    pub _wiremock_server_uri: Option<String>,
    /// Leaked wiremock server ref for request inspection (skills e2e)
    pub wiremock_server_ref: Option<&'static wiremock::MockServer>,
    /// Transcription result from voice scenarios
    pub transcription_result: Option<Result<TranscriptionResult, String>>,
    /// Temp directory handle (kept alive so the dir isn't deleted)
    pub _temp_dir: Option<TempDir>,
    /// Additional temp dirs (kept alive for sandbox hardening symlink tests etc.)
    pub _extra_temp_dirs: Vec<TempDir>,
    /// Exec tool for direct exec tool testing (timeout, env sanitization)
    pub exec_tool: Option<Arc<ExecTool>>,
    /// Environment variable overrides for exec tool env sanitization tests
    pub exec_env_vars: HashMap<String, String>,
    /// Gateway provider wiring: resolved API key for a provider
    pub gateway_resolved_api_key: Option<String>,
    /// Gateway provider readiness report
    pub gateway_readiness_report: Option<Vec<String>>,
    /// Gateway config for provider wiring tests
    pub gateway_config: Option<Config>,
    /// Gateway credential store for wiring tests
    pub gateway_credential_store: Option<CredentialStore>,
    /// Gateway credential snapshot (loaded once, shared across resolution steps)
    pub gateway_cred_snapshot: Option<std::collections::HashMap<String, Credential>>,
    /// Pending tool call from "the mock LLM first returns a tool call" (paired with "then returns text")
    pub pending_tool_call: Option<(String, String)>,
    /// Pending parallel tool calls (name, args_json) for the parallel-then-text step
    pub pending_parallel_calls: Option<Vec<(String, String)>>,
    /// Whether QUECTO_BASE_DIR env var was set by this scenario (needs cleanup)
    pub env_base_dir_set: bool,
    /// Wiremock URI for Anthropic mock (dual-provider scenarios)
    pub wiremock_anthropic_uri: Option<String>,
    /// Subprocess exit code (from spawning quecto as a child process)
    pub subprocess_exit_code: Option<i32>,
    /// Subprocess captured stdout
    pub subprocess_stdout: Option<String>,
    /// Subprocess captured stderr
    pub subprocess_stderr: Option<String>,
    /// REPL: accumulated input lines (built up by "I type" steps)
    pub repl_input_lines: Vec<String>,
    /// REPL: flags to pass (built up by "with flags" steps)
    pub repl_flags: Vec<String>,
    /// REPL: whether the REPL has been executed (lazy execution)
    pub repl_executed: bool,
    /// Bot command response from handle_bot_command()
    pub bot_command_response: Option<Option<String>>,
    /// Whether the gateway shutdown completed cleanly
    pub gateway_shutdown_clean: Option<bool>,
    /// Mock agent for gateway cron/heartbeat scenarios (Debug-opaque wrapper)
    pub _gateway_mock_agent: Option<DebugAgent>,
    /// Mock agent: captured user messages (for gateway cron/heartbeat scenarios)
    pub mock_agent_messages: Arc<Mutex<Vec<String>>>,
    /// Results from execute_cron_tick()
    pub cron_tick_results: Option<Vec<CronJobResult>>,
    /// Results from execute_heartbeat_tick()
    pub heartbeat_tick_results: Option<Vec<HeartbeatTaskResult>>,
    /// In-memory cron store for gateway cron scenarios
    pub gateway_cron_store: Option<Arc<InMemoryCronStore>>,
    /// Config for gateway cron/heartbeat scenarios
    pub gateway_tick_config: Option<Config>,
    /// Captured outbound Telegram texts from real gateway subprocess scenarios
    pub gateway_telegram_outbound_texts: Vec<String>,
    /// Leaked wiremock server ref for web search mock (for mounting responses)
    pub web_search_mock_server: Option<&'static wiremock::MockServer>,
    /// Whether the web search used DDG (for fallback assertion)
    pub web_search_used_ddg: bool,
    /// Voice processing result from application-layer voice pipeline
    pub voice_processing_result: Option<Result<app_voice::VoiceProcessingResult, DomainError>>,
    /// Outbound response text for voice gateway scenarios
    pub voice_bot_response: Option<String>,
    /// Pending CLI args for interactive auth scenarios (set by "I start quecto")
    pub pending_cli_args: Option<Vec<String>>,
    /// Health server address (host:port) for observability scenarios
    pub health_server_addr: Option<String>,
    /// Health server readiness control
    pub health_readiness: Option<Arc<StaticReadiness>>,
    /// HTTP response status from health server request
    pub health_response_status: Option<u16>,
    /// HTTP response body from health server request
    pub health_response_body: Option<String>,
    /// Captured tracing log output for observability scenarios
    pub captured_log_output: Option<Arc<Mutex<String>>>,
    /// Streaming response from provider streaming scenarios
    pub streaming_response: Option<LlmResponse>,
    /// Gateway subprocess child process handle (for long-running gateway tests)
    pub gateway_child: Option<std::process::Child>,
    /// Health server port assigned for gateway health e2e tests
    pub gateway_health_port: Option<u16>,
    /// Leaked wiremock server for gateway voice/spawn e2e tests (unified Telegram + Groq mock)
    pub gateway_mock_server: Option<&'static wiremock::MockServer>,
    /// Groq Whisper mock server (separate from Telegram mock) for voice e2e tests
    pub groq_mock_server: Option<&'static wiremock::MockServer>,
}

/// Ensure world has a temp dir and CliContext pointing to it.
/// Also cleans up QUECTO_BASE_DIR env var if a previous scenario set it.
fn ensure_temp_dir(world: &mut QuectoWorld) {
    // Clean up env var from a previous scenario (single-threaded BDD runner).
    if world.env_base_dir_set {
        // SAFETY: BDD runner is single-threaded (max_concurrent_scenarios(1)).
        unsafe {
            std::env::remove_var("QUECTO_BASE_DIR");
        }
        world.env_base_dir_set = false;
    }
    if world._temp_dir.is_none() {
        let td = TempDir::new().expect("failed to create temp dir");
        world.cli_context.base_dir = Some(td.path().to_path_buf());
        world._temp_dir = Some(td);
    }
}

fn base_path(world: &QuectoWorld) -> PathBuf {
    world
        .cli_context
        .base_dir
        .clone()
        .expect("base_dir should be set")
}

/// Parse a shell-like argument string into individual args.
/// Handles double-quoted and single-quoted strings.
///
/// Sufficient for the hardcoded Gherkin step strings used in BDD
/// scenarios. Does not handle backslash escapes or nested quoting.
///
/// Uses byte-index scanning instead of `.chars()` iteration.
fn shell_split(s: &str) -> Vec<String> {
    let mut args = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        // Skip whitespace
        if bytes[i] == b' ' {
            i += 1;
            continue;
        }

        let mut current = String::new();

        // Quoted token
        if bytes[i] == b'\'' || bytes[i] == b'"' {
            let quote = bytes[i];
            i += 1;
            while i < bytes.len() && bytes[i] != quote {
                current.push(bytes[i] as char);
                i += 1;
            }
            // Skip closing quote
            if i < bytes.len() {
                i += 1;
            }
        } else {
            // Unquoted token — collect until space or quote
            while i < bytes.len() && bytes[i] != b' ' && bytes[i] != b'\'' && bytes[i] != b'"' {
                current.push(bytes[i] as char);
                i += 1;
            }
        }

        if !current.is_empty() {
            args.push(current);
        }
    }
    args
}

/// Convert a 2-column Gherkin table (key | value) to a JSON object string.
fn table_to_json(table: &gherkin::Table) -> String {
    let obj: serde_json::Value = table
        .rows
        .iter()
        .filter(|row| row.len() >= 2)
        .map(|row| (row[0].trim().to_string(), serde_json::json!(row[1].trim())))
        .collect::<serde_json::Map<String, serde_json::Value>>()
        .into();
    obj.to_string()
}

mod agent_loop_steps;
mod agent_msg_steps;
mod agent_tools_steps;
mod auth_steps;
mod config_steps;
mod cron_steps;
mod e2e_steps;
mod gateway_steps;
mod heartbeat_steps;
mod observability_steps;
mod provider_steps;
mod repl_steps;
mod sandbox_steps;
mod security_steps;
mod session_steps;
mod skills_steps;
mod subagent_steps;
mod telegram_steps;
mod voice_steps;

// Runner
// ===========================================================================

fn main() {
    let real_llm_enabled = std::env::var("QUECTO_REAL_LLM").unwrap_or_default() == "1";
    // Optional tag filter: QUECTO_TAG=real-llm runs only scenarios with that tag.
    let tag_filter = std::env::var("QUECTO_TAG").ok();

    futures::executor::block_on(
        QuectoWorld::cucumber()
            .max_concurrent_scenarios(1)
            .fail_on_skipped()
            .filter_run("tests/features", move |feat, _, sc| {
                // Exclude scenarios explicitly tagged @pending
                if sc.tags.iter().any(|t| t == "pending") {
                    return false;
                }
                // Exclude @real-llm scenarios unless QUECTO_REAL_LLM=1
                if sc.tags.iter().any(|t| t == "real-llm") && !real_llm_enabled {
                    return false;
                }
                // If a tag filter is set, only run matching scenarios
                if let Some(ref tag) = tag_filter {
                    return sc.tags.iter().any(|t| t == tag.as_str());
                }
                // Include if feature or scenario is tagged @wip or @done
                feat.tags.iter().any(|t| t == "wip" || t == "done")
                    || sc.tags.iter().any(|t| t == "wip" || t == "done")
            }),
    );
}
