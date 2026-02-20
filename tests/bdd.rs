#![allow(private_interfaces)]

use cucumber::{World, gherkin, given, then, when};
use quecto::application::agent_loop::AgentLoopImpl;
use quecto::application::heartbeat::{self, HeartbeatResult, HeartbeatTask};
use quecto::application::subagent::{SubagentConfig, SubagentContext, validate_agent_id};
use quecto::domain::agent::{AgentInfo, AgentLoop, AgentResult};
use quecto::domain::cron::{CronJob, CronSchedule, CronStore};
use quecto::domain::error::DomainError;
use quecto::domain::message::{LlmResponse, Message, Role, ToolCall};
use quecto::domain::provider::LlmProvider;
use quecto::domain::session::{Session, SessionStore};
use quecto::domain::skill::{Skill, SkillLoader, SkillSource};
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
use quecto::infrastructure::persistence::cron_store::{self, FileCronStore};
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
use quecto::infrastructure::voice::groq_whisper::{GroqWhisperClient, TranscriptionResult};
use quecto::interface::cli::{self, CliContext};
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
    /// Skill loader for skills scenarios
    pub skill_loader_workspace: Option<PathBuf>,
    pub skill_loader_global: Option<PathBuf>,
    pub skill_loader_builtin: Option<PathBuf>,
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

// ===========================================================================
// Config Steps (Given)
// ===========================================================================

#[given(expr = "a config file at {string} with content:")]
fn given_config_file_at_path(world: &mut QuectoWorld, step: &gherkin::Step, _path: String) {
    let content = step.docstring().expect("step should have a docstring");
    ensure_temp_dir(world);
    let config_file = base_path(world).join("config.json");
    std::fs::write(&config_file, content).expect("failed to write config file");
    world.config_path = Some(config_file.to_string_lossy().to_string());
}

#[given(expr = "an environment variable {string} set to {string}")]
fn given_env_var(world: &mut QuectoWorld, key: String, value: String) {
    world.env_overrides.insert(key, value);
}

#[given(expr = "a config file with model {string}")]
fn given_config_file_with_model(world: &mut QuectoWorld, model: String) {
    let content = format!(
        r#"{{
  "agents": {{
    "defaults": {{
      "model": "{model}"
    }}
  }}
}}"#
    );
    ensure_temp_dir(world);
    let config_file = base_path(world).join("config.json");
    std::fs::write(&config_file, content).expect("failed to write config file");
    world.config_path = Some(config_file.to_string_lossy().to_string());
}

#[given(expr = "a config with workspace {string}")]
fn given_config_with_workspace(world: &mut QuectoWorld, workspace: String) {
    let content = format!(
        r#"{{
  "agents": {{
    "defaults": {{
      "workspace": "{workspace}"
    }}
  }}
}}"#
    );
    ensure_temp_dir(world);
    let config_file = base_path(world).join("config.json");
    std::fs::write(&config_file, content).expect("failed to write config file");
    world.config_path = Some(config_file.to_string_lossy().to_string());
}

// ===========================================================================
// Onboard Steps (Given)
// ===========================================================================

#[given(expr = "no config file exists at {string}")]
fn given_no_config(world: &mut QuectoWorld, _path: String) {
    // Create a fresh temp dir with no config file
    let td = TempDir::new().expect("failed to create temp dir");
    world.cli_context.base_dir = Some(td.path().to_path_buf());
    world._temp_dir = Some(td);
    // Verify no config exists
    assert!(!base_path(world).join("config.json").exists());
}

#[given(expr = "a config file already exists at {string}")]
fn given_config_already_exists(world: &mut QuectoWorld, _path: String) {
    let td = TempDir::new().expect("failed to create temp dir");
    // Create a config file
    std::fs::write(td.path().join("config.json"), "{}").expect("failed to write");
    world.cli_context.base_dir = Some(td.path().to_path_buf());
    world._temp_dir = Some(td);
}

// ===========================================================================
// Config Steps (When)
// ===========================================================================

#[when("I load the config")]
fn when_load_config(world: &mut QuectoWorld) {
    let path = world
        .config_path
        .as_ref()
        .expect("config_path must be set before loading");
    let config =
        Config::load_with_env(path, &world.env_overrides).expect("Config::load_with_env failed");
    world.config = Some(config);
}

#[when("I resolve the workspace path")]
fn when_resolve_workspace(world: &mut QuectoWorld) {
    let path = world
        .config_path
        .as_ref()
        .expect("config_path must be set before resolving workspace");
    let config = Config::load(path).expect("Config::load failed");
    world.resolved_workspace = Some(config.workspace_path());
}

// ===========================================================================
// Config Steps (Then)
// ===========================================================================

#[then(expr = "the model should be {string}")]
fn then_model_should_be(world: &mut QuectoWorld, expected: String) {
    let config = world.config.as_ref().expect("config not loaded");
    assert_eq!(config.agents.defaults.model, expected);
}

#[then(expr = "the max_tokens should be {int}")]
fn then_max_tokens_should_be(world: &mut QuectoWorld, expected: u32) {
    let config = world.config.as_ref().expect("config not loaded");
    assert_eq!(config.agents.defaults.max_tokens, expected);
}

#[then(expr = "the OpenAI API key should be {string}")]
fn then_openai_key_should_be(world: &mut QuectoWorld, expected: String) {
    let config = world.config.as_ref().expect("config not loaded");
    assert_eq!(config.providers.openai.api_key, expected);
}

#[then(expr = "the temperature should be {float}")]
fn then_temperature_should_be(world: &mut QuectoWorld, expected: f32) {
    let config = world.config.as_ref().expect("config not loaded");
    assert!(
        (config.agents.defaults.temperature - expected).abs() < f32::EPSILON,
        "expected temperature {}, got {}",
        expected,
        config.agents.defaults.temperature
    );
}

#[then(expr = "the workspace should be {string}")]
fn then_workspace_should_be(world: &mut QuectoWorld, expected: String) {
    let config = world.config.as_ref().expect("config not loaded");
    assert_eq!(config.agents.defaults.workspace, expected);
}

#[then(expr = "the workspace path should start with {string}")]
fn then_workspace_starts_with(world: &mut QuectoWorld, prefix: String) {
    let ws = world
        .resolved_workspace
        .as_ref()
        .expect("resolved_workspace not set");
    assert!(
        ws.starts_with(&prefix),
        "expected workspace '{}' to start with '{}'",
        ws,
        prefix
    );
}

#[then(expr = "the workspace path should end with {string}")]
fn then_workspace_ends_with(world: &mut QuectoWorld, suffix: String) {
    let ws = world
        .resolved_workspace
        .as_ref()
        .expect("resolved_workspace not set");
    assert!(
        ws.ends_with(&suffix),
        "expected workspace '{}' to end with '{}'",
        ws,
        suffix
    );
}

// ===========================================================================
// CLI Steps
// ===========================================================================

#[when("I run quecto with no arguments")]
fn when_run_no_args(world: &mut QuectoWorld) {
    let output = cli::run_with_output(vec!["quecto".to_string()], &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

#[when(expr = "I run quecto with arguments {string}")]
fn when_run_with_args(world: &mut QuectoWorld, args_str: String) {
    let mut args = vec!["quecto".to_string()];
    // Simple shell-like splitting (handles quoted strings)
    let mut current = String::new();
    let mut in_quotes = false;
    for ch in args_str.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ' ' if !in_quotes => {
                if !current.is_empty() {
                    args.push(current.clone());
                    current.clear();
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        args.push(current);
    }
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

#[then(expr = "the exit code should be {int}")]
fn then_exit_code(world: &mut QuectoWorld, expected: i32) {
    assert_eq!(
        world.exit_code, expected,
        "expected exit code {}, got {}.\nstdout: {}\nstderr: {}",
        expected, world.exit_code, world.stdout, world.stderr
    );
}

#[then(expr = "the output should contain {string}")]
fn then_output_contains(world: &mut QuectoWorld, expected: String) {
    let combined = format!("{}{}", world.stdout, world.stderr);
    assert!(
        combined.contains(&expected),
        "expected output to contain '{}', got:\nstdout: {}\nstderr: {}",
        expected,
        world.stdout,
        world.stderr
    );
}

#[then(expr = "the stderr should contain {string}")]
fn then_stderr_contains(world: &mut QuectoWorld, expected: String) {
    assert!(
        world.stderr.contains(&expected),
        "expected stderr to contain '{}', got: {}",
        expected,
        world.stderr
    );
}

#[then(expr = "the output should match {string}")]
fn then_output_matches(world: &mut QuectoWorld, pattern: String) {
    let combined = format!("{}{}", world.stdout, world.stderr);
    let re = regex::Regex::new(&pattern).expect("invalid regex pattern");
    assert!(
        re.is_match(&combined),
        "expected output to match '{}', got:\n{}",
        pattern,
        combined
    );
}

// ===========================================================================
// Onboard Steps (Then)
// ===========================================================================

#[then(expr = "a config file should exist at {string}")]
fn then_config_file_exists(world: &mut QuectoWorld, _path: String) {
    let config_path = base_path(world).join("config.json");
    assert!(
        config_path.exists(),
        "config file should exist at {}",
        config_path.display()
    );
}

#[then(expr = "a workspace directory should exist at {string}")]
fn then_workspace_dir_exists(world: &mut QuectoWorld, _path: String) {
    let ws_path = base_path(world).join("workspace");
    assert!(
        ws_path.is_dir(),
        "workspace dir should exist at {}",
        ws_path.display()
    );
}

#[then(expr = "the workspace should contain {string}")]
fn then_workspace_contains_file(world: &mut QuectoWorld, filename: String) {
    let file_path = base_path(world).join("workspace").join(&filename);
    assert!(
        file_path.exists(),
        "workspace should contain '{}' at {}",
        filename,
        file_path.display()
    );
}

#[then(expr = "the config should have model {string}")]
fn then_config_should_have_model(world: &mut QuectoWorld, expected: String) {
    let config_path = base_path(world).join("config.json");
    let config = Config::load(config_path.to_str().unwrap()).expect("load config");
    assert_eq!(config.agents.defaults.model, expected);
}

#[then(expr = "the config should have max_tokens {int}")]
fn then_config_should_have_max_tokens(world: &mut QuectoWorld, expected: u32) {
    let config_path = base_path(world).join("config.json");
    let config = Config::load(config_path.to_str().unwrap()).expect("load config");
    assert_eq!(config.agents.defaults.max_tokens, expected);
}

#[then(expr = "the config should have temperature {float}")]
fn then_config_should_have_temperature(world: &mut QuectoWorld, expected: f32) {
    let config_path = base_path(world).join("config.json");
    let config = Config::load(config_path.to_str().unwrap()).expect("load config");
    assert!(
        (config.agents.defaults.temperature - expected).abs() < f32::EPSILON,
        "expected temperature {}, got {}",
        expected,
        config.agents.defaults.temperature
    );
}

#[then(expr = "the config should have restrict_to_workspace {word}")]
fn then_config_should_have_restrict(world: &mut QuectoWorld, expected: String) {
    let config_path = base_path(world).join("config.json");
    let config = Config::load(config_path.to_str().unwrap()).expect("load config");
    let expected_bool = expected == "true";
    assert_eq!(
        config.agents.defaults.restrict_to_workspace, expected_bool,
        "expected restrict_to_workspace {}, got {}",
        expected_bool, config.agents.defaults.restrict_to_workspace
    );
}

// ===========================================================================
// Security / Sandbox Steps
// ===========================================================================

#[given(expr = "a sandboxed workspace at {string}")]
fn given_sandboxed_workspace(world: &mut QuectoWorld, path: String) {
    let ws = PathBuf::from(&path);
    // Default to restrict_to_workspace = true; can be overridden by next step
    world.sandbox = Some(Sandbox::new(Some(ws), true));
}

#[given(expr = "restrict_to_workspace is {word}")]
fn given_restrict_to_workspace(world: &mut QuectoWorld, value: String) {
    let restrict = value == "true";
    if let Some(ref mut sb) = world.sandbox {
        sb.restrict_to_workspace = restrict;
    } else {
        world.sandbox = Some(Sandbox::new(None, restrict));
    }
}

#[when(expr = "the agent tries to validate path {string}")]
fn when_validate_path(world: &mut QuectoWorld, path: String) {
    let sb = world.sandbox.as_ref().expect("sandbox not configured");
    world.validation_result = Some(
        sb.validate_path(&path)
            .map(|_| ())
            .map_err(|e| e.to_string()),
    );
}

#[when(expr = "the agent tries to validate command {string}")]
fn when_validate_command(world: &mut QuectoWorld, command: String) {
    let default_sb = Sandbox::new(None, false);
    let sb = world.sandbox.as_ref().unwrap_or(&default_sb);
    world.validation_result = Some(sb.validate_command(&command).map_err(|e| e.to_string()));
}

#[then("the validation should be an error")]
fn then_validation_is_error(world: &mut QuectoWorld) {
    let result = world
        .validation_result
        .as_ref()
        .expect("no validation result");
    assert!(result.is_err(), "expected validation error, got Ok");
}

#[then("the validation should be ok")]
fn then_validation_is_ok(world: &mut QuectoWorld) {
    let result = world
        .validation_result
        .as_ref()
        .expect("no validation result");
    assert!(
        result.is_ok(),
        "expected validation to succeed, got: {}",
        result.as_ref().unwrap_err()
    );
}

#[then(expr = "the error should mention {string}")]
fn then_error_should_mention(world: &mut QuectoWorld, expected: String) {
    let result = world
        .validation_result
        .as_ref()
        .expect("no validation result");
    let err_msg = result.as_ref().unwrap_err();
    assert!(
        err_msg.contains(&expected),
        "expected error to mention '{}', got: {}",
        expected,
        err_msg
    );
}

// ===========================================================================
// Agent Tools Steps
// ===========================================================================

#[given("a tool workspace")]
fn given_tool_workspace(world: &mut QuectoWorld) {
    let td = TempDir::new().expect("failed to create temp dir");
    let ws = td.path().to_path_buf();
    let sandbox = Sandbox::new(Some(ws.clone()), true);
    let registry = ToolRegistryImpl::with_core_tools(ws.clone(), sandbox);
    world.tool_workspace = Some(ws);
    world.tool_registry = Some(registry);
    world._temp_dir = Some(td);
}

#[given(expr = "a file {string} exists with content {string}")]
fn given_file_exists(world: &mut QuectoWorld, filename: String, content: String) {
    let ws = world
        .tool_workspace
        .as_ref()
        .expect("tool workspace not set");
    let path = ws.join(&filename);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent dirs");
    }
    std::fs::write(&path, &content).expect("write file");
}

#[when(expr = "the agent executes tool {string} with args:")]
fn when_agent_executes_tool(world: &mut QuectoWorld, tool_name: String, step: &gherkin::Step) {
    let table = step.table.as_ref().expect("step should have a table");
    // Build JSON from table: first column is key, second is value
    let mut map = serde_json::Map::new();
    for row in &table.rows {
        if row.len() >= 2 {
            map.insert(
                row[0].trim().to_string(),
                serde_json::Value::String(row[1].trim().to_string()),
            );
        }
    }
    let args_json = serde_json::Value::Object(map).to_string();

    let registry = world.tool_registry.as_ref().expect("tool registry not set");

    // Run the tool using a tokio runtime
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(registry.execute(&tool_name, &args_json));

    world.tool_result = Some(result.map_err(|e| e.to_string()));
}

#[then(expr = "the tool result should contain {string}")]
fn then_tool_result_contains(world: &mut QuectoWorld, expected: String) {
    let result = world.tool_result.as_ref().expect("no tool result");
    match result {
        Ok(tr) => assert!(
            tr.content.contains(&expected),
            "expected tool result to contain '{}', got: {}",
            expected,
            tr.content
        ),
        Err(e) => panic!("tool returned error: {}", e),
    }
}

#[then("the tool result should not be an error")]
fn then_tool_result_not_error(world: &mut QuectoWorld) {
    let result = world.tool_result.as_ref().expect("no tool result");
    match result {
        Ok(tr) => assert!(
            !tr.is_error,
            "expected tool result to not be an error, content: {}",
            tr.content
        ),
        Err(e) => panic!("tool returned DomainError: {}", e),
    }
}

#[then(expr = "the file {string} should exist in the workspace")]
fn then_file_exists_in_workspace(world: &mut QuectoWorld, filename: String) {
    let ws = world
        .tool_workspace
        .as_ref()
        .expect("tool workspace not set");
    let path = ws.join(&filename);
    assert!(
        path.exists(),
        "file '{}' should exist at {}",
        filename,
        path.display()
    );
}

#[then(expr = "the file {string} should contain {string}")]
fn then_file_contains(world: &mut QuectoWorld, filename: String, expected: String) {
    let ws = world
        .tool_workspace
        .as_ref()
        .expect("tool workspace not set");
    let path = ws.join(&filename);
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("failed to read {}", path.display()));
    assert!(
        content.contains(&expected),
        "expected '{}' to contain '{}', got: {}",
        filename,
        expected,
        content
    );
}

#[then(expr = "the tool registry should contain {string}")]
fn then_registry_contains(world: &mut QuectoWorld, tool_name: String) {
    let registry = world.tool_registry.as_ref().expect("tool registry not set");
    let names = registry.names();
    assert!(
        names.contains(&tool_name),
        "registry should contain '{}', has: {:?}",
        tool_name,
        names
    );
}

// ===========================================================================
// Security (Subagent/Heartbeat Inheritance) Steps
// ===========================================================================

#[given("a subagent context inheriting restrict_to_workspace")]
fn given_subagent_inheriting_sandbox(world: &mut QuectoWorld) {
    let sb = world.sandbox.as_ref().expect("sandbox not configured");
    // Create a subagent config that inherits the sandbox's restrict_to_workspace
    world.subagent_config = Some(SubagentConfig {
        task: "test task".to_string(),
        agent_id: None,
        restrict_to_workspace: sb.restrict_to_workspace,
        deliver_to: None,
    });
    let ctx = SubagentContext::from_config(world.subagent_config.as_ref().unwrap());
    world.subagent_context = Some(ctx);
}

#[when(expr = "the subagent sandbox validates path {string}")]
fn when_subagent_validates_path(world: &mut QuectoWorld, path: String) {
    // The subagent inherits the same sandbox config; validate using it
    let sb = world.sandbox.as_ref().expect("sandbox not configured");
    // Verify the subagent context also has restrict_to_workspace set
    let ctx = world
        .subagent_context
        .as_ref()
        .expect("subagent context not set");
    assert_eq!(ctx.restrict_to_workspace, sb.restrict_to_workspace);
    world.validation_result = Some(
        sb.validate_path(&path)
            .map(|_| ())
            .map_err(|e| e.to_string()),
    );
}

#[when(expr = "a heartbeat sandbox validates path {string}")]
fn when_heartbeat_validates_path(world: &mut QuectoWorld, path: String) {
    // Heartbeat tasks run within the same sandbox restrictions
    let sb = world.sandbox.as_ref().expect("sandbox not configured");
    world.validation_result = Some(
        sb.validate_path(&path)
            .map(|_| ())
            .map_err(|e| e.to_string()),
    );
}

// ===========================================================================
// Sandbox Hardening Steps
// ===========================================================================

#[given("a sandboxed workspace at a temporary directory")]
fn given_sandboxed_workspace_temp(world: &mut QuectoWorld) {
    let td = TempDir::new().expect("failed to create temp dir");
    let ws = td.path().to_path_buf();
    world.sandbox = Some(Sandbox::new(Some(ws.clone()), true));
    world.tool_workspace = Some(ws);
    world._extra_temp_dirs.push(td);
}

#[given(expr = "a symlink {string} in the workspace pointing to {string}")]
fn given_symlink_in_workspace(world: &mut QuectoWorld, link_name: String, target: String) {
    let ws = world.tool_workspace.as_ref().expect("workspace not set");
    let link_path = ws.join(&link_name);
    // If target is relative, it should be relative to the workspace
    let target_path = if target.starts_with('/') {
        PathBuf::from(&target)
    } else {
        ws.join(&target)
    };
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target_path, &link_path).unwrap_or_else(|e| {
        panic!(
            "failed to create symlink {} -> {}: {}",
            link_path.display(),
            target_path.display(),
            e
        )
    });
}

#[given(expr = "a file {string} exists in the workspace")]
fn given_file_exists_in_workspace(world: &mut QuectoWorld, filename: String) {
    let ws = world.tool_workspace.as_ref().expect("workspace not set");
    let path = ws.join(&filename);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent dirs");
    }
    std::fs::write(&path, "test content").expect("write file");
}

#[when(expr = "the agent tries to validate path {string} resolved against the workspace")]
fn when_validate_path_resolved(world: &mut QuectoWorld, path: String) {
    let sb = world.sandbox.as_ref().expect("sandbox not configured");
    let ws = world.tool_workspace.as_ref().expect("workspace not set");
    let full_path = ws.join(&path);
    world.validation_result = Some(
        sb.validate_path(full_path.to_str().unwrap())
            .map(|_| ())
            .map_err(|e| e.to_string()),
    );
}

// --- Allowlist steps ---

#[given(expr = "a sandbox with command allowlist {string}")]
fn given_sandbox_with_allowlist(world: &mut QuectoWorld, allowlist: String) {
    let commands: Vec<String> = if allowlist.is_empty() {
        vec![]
    } else {
        allowlist.split(',').map(|s| s.trim().to_string()).collect()
    };
    let mut sb = Sandbox::new(None, false);
    sb.command_allowlist = Some(commands);
    world.sandbox = Some(sb);
}

#[given("a sandbox without a command allowlist")]
fn given_sandbox_without_allowlist(world: &mut QuectoWorld) {
    let sb = Sandbox::new(None, false);
    // command_allowlist defaults to None
    world.sandbox = Some(sb);
}

// --- Exec timeout steps ---

#[given(expr = "an exec tool with a timeout of {int} seconds")]
fn given_exec_tool_with_timeout(world: &mut QuectoWorld, timeout: u64) {
    let td = TempDir::new().expect("failed to create temp dir");
    let ws = td.path().to_path_buf();
    let sandbox = Sandbox::new(Some(ws.clone()), false);
    let tool = ExecTool::with_timeout(
        Arc::new(ws.clone()),
        Arc::new(sandbox),
        std::time::Duration::from_secs(timeout),
    );
    world.exec_tool = Some(Arc::new(tool));
    world.tool_workspace = Some(ws);
    world._extra_temp_dirs.push(td);
}

#[given("an exec tool with no explicit timeout")]
fn given_exec_tool_no_timeout(world: &mut QuectoWorld) {
    let td = TempDir::new().expect("failed to create temp dir");
    let ws = td.path().to_path_buf();
    let sandbox = Sandbox::new(Some(ws.clone()), false);
    let tool = ExecTool::new(Arc::new(ws.clone()), Arc::new(sandbox));
    world.exec_tool = Some(Arc::new(tool));
    world.tool_workspace = Some(ws);
    world._extra_temp_dirs.push(td);
}

#[when(expr = "the agent executes command {string}")]
fn when_agent_executes_command(world: &mut QuectoWorld, command: String) {
    let tool = world.exec_tool.as_ref().expect("exec tool not set");
    let args = serde_json::json!({"command": command}).to_string();
    let env_vars = world.exec_env_vars.clone();

    let result = tokio::runtime::Runtime::new().unwrap().block_on(async {
        if env_vars.is_empty() {
            tool.execute(&args).await
        } else {
            tool.execute_with_env(&args, &env_vars).await
        }
    });

    world.tool_result = Some(result.map_err(|e| e.to_string()));
}

#[then("the tool result should be an error")]
fn then_tool_result_is_error(world: &mut QuectoWorld) {
    let result = world.tool_result.as_ref().expect("no tool result");
    if let Ok(tr) = result {
        assert!(
            tr.is_error,
            "expected tool result to be an error, got success: {}",
            tr.content
        );
    }
    // Err(_) is also an error — nothing to assert
}

#[then(expr = "the tool result should not contain {string}")]
fn then_tool_result_not_contains(world: &mut QuectoWorld, unexpected: String) {
    let result = world.tool_result.as_ref().expect("no tool result");
    match result {
        Ok(tr) => assert!(
            !tr.content.contains(&unexpected),
            "expected tool result NOT to contain '{}', got: {}",
            unexpected,
            tr.content
        ),
        Err(e) => assert!(
            !e.contains(&unexpected),
            "expected error NOT to contain '{}', got: {}",
            unexpected,
            e
        ),
    }
}

#[then(expr = "the exec tool should have a default timeout of {int} seconds")]
fn then_exec_tool_default_timeout(world: &mut QuectoWorld, expected: u64) {
    let tool = world.exec_tool.as_ref().expect("exec tool not set");
    let actual = tool.timeout().as_secs();
    assert_eq!(
        actual, expected,
        "expected default timeout {}s, got {}s",
        expected, actual
    );
}

// --- Env sanitization steps ---

#[given("an exec tool in a sandboxed workspace")]
fn given_exec_tool_in_sandbox(world: &mut QuectoWorld) {
    let td = TempDir::new().expect("failed to create temp dir");
    let ws = td.path().to_path_buf();
    let sandbox = Sandbox::new(Some(ws.clone()), false);
    let tool = ExecTool::new(Arc::new(ws.clone()), Arc::new(sandbox));
    world.exec_tool = Some(Arc::new(tool));
    world.tool_workspace = Some(ws);
    world.exec_env_vars.clear();
    world._extra_temp_dirs.push(td);
}

#[given(expr = "the environment contains {string} set to {string}")]
fn given_exec_env_var(world: &mut QuectoWorld, key: String, value: String) {
    world.exec_env_vars.insert(key, value);
}

// --- Credential file permission steps ---

#[given("a credential store at a temporary directory")]
fn given_credential_store_at_temp(world: &mut QuectoWorld) {
    let td = TempDir::new().expect("failed to create temp dir");
    let base = td.path().to_path_buf();
    world.credential_store = Some(CredentialStore::new(&base));
    world._extra_temp_dirs.push(td);
}

#[given(expr = "the credentials file exists with permissions {int}")]
fn given_credentials_file_with_permissions(world: &mut QuectoWorld, perms: u32) {
    let store = world
        .credential_store
        .as_ref()
        .expect("credential store not set");
    // Store a dummy credential to create the file
    store
        .store(Credential {
            provider: "dummy".to_string(),
            token: "dummy".to_string(),
            method: AuthMethod::Token,
            expires_at: None,
        })
        .unwrap();
    // Now change the permissions to the specified value (interpret as octal)
    let octal_perms = u32::from_str_radix(&format!("{}", perms), 8)
        .unwrap_or_else(|_| panic!("invalid octal permissions: {}", perms));
    let cred_path = store.path();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = std::fs::Permissions::from_mode(octal_perms);
        std::fs::set_permissions(cred_path, permissions).expect("set permissions");
    }
}

#[then(expr = "the credentials file should have permissions {int}")]
fn then_credentials_file_permissions(world: &mut QuectoWorld, expected: u32) {
    let store = world
        .credential_store
        .as_ref()
        .expect("credential store not set");
    // Interpret the expected value as octal (e.g., 0600 -> 0o600 = 384 decimal)
    let octal_expected = u32::from_str_radix(&format!("{}", expected), 8)
        .unwrap_or_else(|_| panic!("invalid octal permissions: {}", expected));
    let cred_path = store.path();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = std::fs::metadata(cred_path)
            .unwrap_or_else(|e| panic!("failed to read metadata for {:?}: {}", cred_path, e));
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(
            mode, octal_expected,
            "expected permissions {:04o}, got {:04o}",
            octal_expected, mode
        );
    }
}

// ===========================================================================
// Agent Tools (Message, Spawn) Steps
// ===========================================================================

#[given(expr = "a message tool with default target {string}")]
fn given_message_tool(world: &mut QuectoWorld, target: String) {
    let mut bus = MessageBus::new(16);
    let sender = bus.outbound_sender();
    let receiver = bus.take_outbound_receiver().unwrap();
    world.message_bus_receiver = Some(receiver);

    let tool = MessageTool::new(sender, Some(target));
    let mut registry = ToolRegistryImpl::new();
    registry.register(Arc::new(tool));
    world.tool_registry = Some(registry);
}

#[when(expr = "the agent sends a message {string} via the message tool")]
fn when_send_via_message_tool(world: &mut QuectoWorld, text: String) {
    let registry = world.tool_registry.as_ref().expect("tool registry not set");
    let args = serde_json::json!({"text": text}).to_string();
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(registry.execute("message", &args))
        .unwrap();
    world.tool_result = Some(Ok(result));
}

#[then(expr = "the outbound bus should have a message for {string} with text {string}")]
fn then_outbound_bus_has_message(world: &mut QuectoWorld, target: String, text: String) {
    let receiver = world
        .message_bus_receiver
        .as_mut()
        .expect("no bus receiver");
    let msg = receiver.try_recv().expect("no message on outbound bus");
    assert_eq!(
        msg.target, target,
        "expected target '{}', got '{}'",
        target, msg.target
    );
    assert_eq!(
        msg.text, text,
        "expected text '{}', got '{}'",
        text, msg.text
    );
}

#[given(expr = "a spawn tool with allowed agents {string} and {string}")]
fn given_spawn_tool(world: &mut QuectoWorld, agent1: String, agent2: String) {
    world.spawn_tool = Some(SpawnTool::new(vec![agent1, agent2], true));
}

#[when(expr = "the agent executes the spawn tool with task {string}")]
fn when_execute_spawn_tool(world: &mut QuectoWorld, task: String) {
    let tool = world.spawn_tool.as_ref().expect("spawn tool not set");
    let args = serde_json::json!({"task": task}).to_string();
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(tool.execute(&args))
        .unwrap();
    world.spawn_result = Some(result);
}

#[when(expr = "the agent executes the spawn tool with task {string} and agent_id {string}")]
fn when_execute_spawn_with_agent(world: &mut QuectoWorld, task: String, agent_id: String) {
    let tool = world.spawn_tool.as_ref().expect("spawn tool not set");
    let args = serde_json::json!({"task": task, "agent_id": agent_id}).to_string();
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(tool.execute(&args))
        .unwrap();
    world.spawn_result = Some(result);
}

#[then("the spawn result should confirm the subagent was spawned")]
fn then_spawn_result_ok(world: &mut QuectoWorld) {
    let result = world.spawn_result.as_ref().expect("no spawn result");
    assert!(
        !result.is_error,
        "expected spawn success, got error: {}",
        result.content
    );
    assert!(
        result.content.contains("spawned"),
        "expected 'spawned' in content: {}",
        result.content
    );
}

#[then(expr = "the spawn result should be an error mentioning {string}")]
fn then_spawn_result_error(world: &mut QuectoWorld, expected: String) {
    let result = world.spawn_result.as_ref().expect("no spawn result");
    assert!(result.is_error, "expected spawn error");
    assert!(
        result.content.contains(&expected),
        "expected error to mention '{}', got: {}",
        expected,
        result.content
    );
}

// ===========================================================================
// Provider Steps
// ===========================================================================

#[given(expr = "a config with provider {string} and api_key {string}")]
fn given_provider_config(world: &mut QuectoWorld, provider_name: String, api_key: String) {
    world.provider = providers::create_provider(&provider_name, api_key, None);
}

#[when("I create a provider from config")]
fn when_create_provider(world: &mut QuectoWorld) {
    // Provider was already created in the Given step
    assert!(
        world.provider.is_some(),
        "provider should have been created"
    );
}

#[then(expr = "the provider should be {string}")]
fn then_provider_is(world: &mut QuectoWorld, expected: String) {
    let provider = world.provider.as_ref().expect("no provider created");
    assert_eq!(provider.name(), expected);
}

#[given(expr = "a provider error with status {int}")]
fn given_provider_error(world: &mut QuectoWorld, status: u16) {
    world.error_class = Some(ErrorClass::from_status(status));
}

#[then(expr = "the error should be classified as {string}")]
fn then_error_classified_as(world: &mut QuectoWorld, expected: String) {
    let class = world.error_class.as_ref().expect("no error class");
    assert_eq!(
        class.as_str(),
        expected,
        "expected error class '{}', got '{}'",
        expected,
        class.as_str()
    );
}

#[then("the error should be retryable")]
fn then_error_retryable(world: &mut QuectoWorld) {
    let class = world.error_class.as_ref().expect("no error class");
    assert!(class.is_retryable(), "expected error to be retryable");
}

#[then("the error should not be retryable")]
fn then_error_not_retryable(world: &mut QuectoWorld) {
    let class = world.error_class.as_ref().expect("no error class");
    assert!(!class.is_retryable(), "expected error to not be retryable");
}

// ===========================================================================
// Provider Fallback Steps
// ===========================================================================

/// A simple mock provider for BDD fallback tests that either succeeds or fails.
#[derive(Debug)]
struct BddTestProvider {
    provider_name: String,
    result: Mutex<Result<LlmResponse, String>>,
}

impl BddTestProvider {
    fn succeeding(name: &str, content: &str) -> Arc<Self> {
        Arc::new(Self {
            provider_name: name.to_string(),
            result: Mutex::new(Ok(LlmResponse {
                content: Some(content.to_string()),
                tool_calls: vec![],
                usage: None,
            })),
        })
    }

    fn failing(name: &str, error: &str) -> Arc<Self> {
        Arc::new(Self {
            provider_name: name.to_string(),
            result: Mutex::new(Err(error.to_string())),
        })
    }
}

impl LlmProvider for BddTestProvider {
    fn name(&self) -> &str {
        &self.provider_name
    }

    fn chat(
        &self,
        _request: quecto::domain::provider::ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        let result = self.result.lock().unwrap().clone();
        Box::pin(async move {
            match result {
                Ok(r) => Ok(r),
                Err(e) => Err(DomainError::Provider(e)),
            }
        })
    }
}

/// World fields for storing the primary/fallback providers before building FallbackProvider.
/// We store them as Vec since the FallbackProvider takes a vec.
static FALLBACK_PROVIDERS_KEY: &str = "_fallback_providers";

#[given(expr = "a primary provider that returns a server error {string}")]
fn given_primary_fails_server(world: &mut QuectoWorld, error: String) {
    let primary = BddTestProvider::failing("openai", &error) as Arc<dyn LlmProvider>;
    // Store in env_overrides as a sentinel; actual providers stored differently
    world
        .env_overrides
        .insert(FALLBACK_PROVIDERS_KEY.to_string(), "set".to_string());
    // We'll rebuild when creating the fallback provider
    world.provider = Some(primary);
}

#[given(expr = "a primary provider that returns a rate limit error {string}")]
fn given_primary_fails_rate_limit(world: &mut QuectoWorld, error: String) {
    let primary = BddTestProvider::failing("openai", &error) as Arc<dyn LlmProvider>;
    world
        .env_overrides
        .insert(FALLBACK_PROVIDERS_KEY.to_string(), "set".to_string());
    world.provider = Some(primary);
}

#[given(expr = "a fallback provider that returns {string}")]
fn given_fallback_that_returns(world: &mut QuectoWorld, content: String) {
    let primary = world
        .provider
        .take()
        .expect("primary provider must be set first");
    let fallback = BddTestProvider::succeeding("anthropic", &content) as Arc<dyn LlmProvider>;
    let fp = FallbackProvider::new(vec![primary, fallback]).with_cooldown_secs(60);
    world.fallback_provider = Some(Arc::new(fp));
}

#[when("I send a chat request through the fallback provider")]
fn when_send_through_fallback(world: &mut QuectoWorld) {
    let fp = world
        .fallback_provider
        .as_ref()
        .expect("fallback provider not set");
    let messages = vec![Message {
        role: Role::User,
        content: "test".to_string(),
        tool_calls: vec![],
        tool_call_id: None,
    }];
    let req = quecto::domain::provider::ChatRequest {
        messages: &messages,
        tools: &[],
        model: "test-model",
        max_tokens: 1024,
        temperature: 0.7,
    };
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(fp.chat(req))
        .expect("fallback chat should succeed");
    world.fallback_response = Some(result);
}

#[when("I send a second chat request through the fallback provider")]
fn when_send_second_through_fallback(world: &mut QuectoWorld) {
    // Same as above — the primary should be on cooldown, so it goes straight to fallback
    let fp = world
        .fallback_provider
        .as_ref()
        .expect("fallback provider not set");
    let messages = vec![Message {
        role: Role::User,
        content: "second test".to_string(),
        tool_calls: vec![],
        tool_call_id: None,
    }];
    let req = quecto::domain::provider::ChatRequest {
        messages: &messages,
        tools: &[],
        model: "test-model",
        max_tokens: 1024,
        temperature: 0.7,
    };
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(fp.chat(req))
        .expect("fallback chat should succeed on second call");
    world.fallback_response = Some(result);
}

#[then(expr = "the fallback response content should be {string}")]
fn then_fallback_response_content(world: &mut QuectoWorld, expected: String) {
    let response = world
        .fallback_response
        .as_ref()
        .expect("no fallback response");
    let content = response.content.as_ref().expect("response has no content");
    assert_eq!(
        content, &expected,
        "expected fallback response '{}', got '{}'",
        expected, content
    );
}

// ===========================================================================
// Provider Mock Server Steps (for real HTTP chat testing)
// ===========================================================================

#[given("an OpenAI provider with a mock server")]
fn given_openai_with_mock_server(world: &mut QuectoWorld) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(wiremock::MockServer::start());
    let uri = server.uri();

    // Provider created but will be replaced when mock response is configured
    world.provider = Some(Arc::new(
        quecto::infrastructure::providers::openai::OpenAiProvider::new(
            "sk-test-key".to_string(),
            Some(uri.clone()),
        ),
    ));
    world._wiremock_server_uri = Some(uri);
    std::mem::forget(server);
    std::mem::forget(rt);
}

#[given(expr = "the mock server returns a chat response with content {string}")]
fn given_mock_chat_response(world: &mut QuectoWorld, content: String) {
    // Create a fresh server with the mock already mounted
    let rt2 = tokio::runtime::Runtime::new().unwrap();
    let (uri2, _server2) = rt2.block_on(async {
        let server = wiremock::MockServer::start().await;
        let response_body = serde_json::json!({
            "id": "chatcmpl-test",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": content
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5,
                "total_tokens": 15
            }
        });
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .and(wiremock::matchers::header(
                "Authorization",
                "Bearer sk-test-key",
            ))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(response_body))
            .mount(&server)
            .await;
        let uri = server.uri();
        (uri, server)
    });

    // Recreate provider pointing at this mock
    world.provider = Some(Arc::new(
        quecto::infrastructure::providers::openai::OpenAiProvider::new(
            "sk-test-key".to_string(),
            Some(uri2.clone()),
        ),
    ));
    world._wiremock_server_uri = Some(uri2);
    std::mem::forget(_server2);
    std::mem::forget(rt2);
}

#[when(expr = "I send a chat request with message {string} and a tool {string}")]
fn when_send_chat_with_tool(world: &mut QuectoWorld, message: String, tool_name: String) {
    let provider = world.provider.as_ref().expect("provider not set");
    let messages = vec![Message {
        role: Role::User,
        content: message,
        tool_calls: vec![],
        tool_call_id: None,
    }];
    let tools = vec![quecto::domain::tool::ToolDefinition {
        name: tool_name,
        description: "Execute a command".to_string(),
        parameters_schema: r#"{"type":"object","properties":{"command":{"type":"string"}}}"#
            .to_string(),
    }];
    let req = quecto::domain::provider::ChatRequest {
        messages: &messages,
        tools: &tools,
        model: "gpt-4",
        max_tokens: 1024,
        temperature: 0.7,
    };
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(provider.chat(req));
    match result {
        Ok(response) => {
            world.fallback_response = Some(response);
        }
        Err(e) => {
            panic!("chat request failed: {}", e);
        }
    }
}

#[then(expr = "the chat response content should be {string}")]
fn then_chat_response_content(world: &mut QuectoWorld, expected: String) {
    let response = world.fallback_response.as_ref().expect("no chat response");
    let content = response.content.as_ref().expect("response has no content");
    assert_eq!(
        content, &expected,
        "expected chat response '{}', got '{}'",
        expected, content
    );
}

#[then("the chat request should have included an Authorization header")]
fn then_chat_had_auth_header(world: &mut QuectoWorld) {
    // The mock server requires an exact `Authorization: Bearer sk-test-key` header
    // (via wiremock::matchers::header on the mock setup). If the provider omits or
    // sends the wrong header, the mock returns no match and the request fails.
    // A successful response with content therefore proves the header was sent.
    let response = world
        .fallback_response
        .as_ref()
        .expect("no chat response — provider may not have sent the Authorization header");
    assert!(
        response.content.is_some(),
        "mock server requires Authorization header; no content means the header was missing or wrong"
    );
}

// ===========================================================================
// Agent Loop Steps
// ===========================================================================

/// Helper: ensure a mock LLM provider is created and a basic agent loop
/// can be built. Returns the mock provider (for queuing responses).
fn ensure_mock_llm(world: &mut QuectoWorld) -> Arc<MockLlmProvider> {
    if world.mock_llm.is_none() {
        world.mock_llm = Some(Arc::new(MockLlmProvider::new()));
    }
    world.mock_llm.clone().unwrap()
}

/// Helper: build an AgentLoopImpl from the world's current state.
fn build_agent_loop(world: &QuectoWorld, max_iterations: Option<u32>) -> AgentLoopImpl {
    let provider = world.mock_llm.clone().expect("mock LLM not configured") as Arc<dyn LlmProvider>;

    // Build a tool registry from mock_tools or tool_registry
    let registry = if !world.mock_tools.is_empty() {
        let mut reg = ToolRegistryImpl::new();
        for tool in world.mock_tools.values() {
            reg.register(tool.clone());
        }
        reg
    } else if let Some(ref reg) = world.tool_registry {
        // We can't clone ToolRegistryImpl, so build a new empty one for scenarios
        // that don't need tools.
        let _ = reg;
        ToolRegistryImpl::new()
    } else {
        ToolRegistryImpl::new()
    };

    let mut agent = AgentLoopImpl::new(quecto::application::agent_loop::AgentLoopConfig {
        provider,
        tool_registry: Box::new(registry),
        model: "test-model".to_string(),
        max_tokens: 1024,
        temperature: 0.7,
    });

    if let Some(max) = max_iterations {
        agent = agent.with_max_tool_iterations(max);
    }

    agent
}

#[given("a configured agent with a mock LLM")]
fn given_configured_agent_with_mock(world: &mut QuectoWorld) {
    ensure_mock_llm(world);
}

#[given(expr = "the LLM returns a plain text response {string}")]
fn given_llm_returns_text(world: &mut QuectoWorld, text: String) {
    let mock = ensure_mock_llm(world);
    mock.push_response(LlmResponse {
        content: Some(text),
        tool_calls: vec![],
        usage: None,
    });
}

#[given(expr = "the LLM returns a tool call for {string} with args:")]
fn given_llm_returns_tool_call(world: &mut QuectoWorld, tool_name: String, step: &gherkin::Step) {
    let mock = ensure_mock_llm(world);
    let table = step.table.as_ref().expect("step should have a table");
    let mut map = serde_json::Map::new();
    for row in &table.rows {
        if row.len() >= 2 {
            map.insert(
                row[0].trim().to_string(),
                serde_json::Value::String(row[1].trim().to_string()),
            );
        }
    }
    let args_json = serde_json::Value::Object(map).to_string();
    mock.push_response(LlmResponse {
        content: None,
        tool_calls: vec![ToolCall {
            id: format!("call_{}", tool_name),
            name: tool_name,
            arguments: args_json,
        }],
        usage: None,
    });
}

#[given(expr = "the tool {string} returns {string}")]
fn given_tool_returns(world: &mut QuectoWorld, tool_name: String, response: String) {
    let tool = Arc::new(MockBddTool::new(&tool_name, &response));
    world.mock_tools.insert(tool_name, tool);
}

#[given(expr = "the LLM then returns {string}")]
fn given_llm_then_returns(world: &mut QuectoWorld, text: String) {
    let mock = ensure_mock_llm(world);
    mock.push_response(LlmResponse {
        content: Some(text),
        tool_calls: vec![],
        usage: None,
    });
}

#[given(expr = "the LLM returns tool calls in sequence: {string}, {string}")]
fn given_llm_returns_tool_calls_in_sequence(world: &mut QuectoWorld, tool1: String, tool2: String) {
    let mock = ensure_mock_llm(world);

    // First call returns tool1
    mock.push_response(LlmResponse {
        content: None,
        tool_calls: vec![ToolCall {
            id: format!("call_{}", tool1),
            name: tool1.clone(),
            arguments: "{}".to_string(),
        }],
        usage: None,
    });

    // Second call returns tool2
    mock.push_response(LlmResponse {
        content: None,
        tool_calls: vec![ToolCall {
            id: format!("call_{}", tool2),
            name: tool2.clone(),
            arguments: "{}".to_string(),
        }],
        usage: None,
    });

    // Third call returns final text
    mock.push_response(LlmResponse {
        content: Some("Done".to_string()),
        tool_calls: vec![],
        usage: None,
    });

    // Register mock tools if not already present
    if !world.mock_tools.contains_key(&tool1) {
        world
            .mock_tools
            .insert(tool1.clone(), Arc::new(MockBddTool::new(&tool1, "ok")));
    }
    if !world.mock_tools.contains_key(&tool2) {
        world
            .mock_tools
            .insert(tool2.clone(), Arc::new(MockBddTool::new(&tool2, "ok")));
    }
}

#[given(expr = "a configured agent with max_tool_iterations {int}")]
fn given_agent_with_max_iterations(world: &mut QuectoWorld, max: u32) {
    ensure_mock_llm(world);
    // Store max iterations; will be used when building the agent
    world
        .env_overrides
        .insert("_max_tool_iterations".to_string(), max.to_string());
}

#[given("the LLM always returns a tool call")]
fn given_llm_always_returns_tool_call(world: &mut QuectoWorld) {
    let mock = ensure_mock_llm(world);
    // Queue many tool call responses (more than any reasonable limit)
    for i in 0..50 {
        mock.push_response(LlmResponse {
            content: None,
            tool_calls: vec![ToolCall {
                id: format!("call_{}", i),
                name: "exec".to_string(),
                arguments: r#"{"command":"echo hi"}"#.to_string(),
            }],
            usage: None,
        });
    }
    // Register the exec mock tool
    if !world.mock_tools.contains_key("exec") {
        world.mock_tools.insert(
            "exec".to_string(),
            Arc::new(MockBddTool::new("exec", "output")),
        );
    }
}

#[given(expr = "a configured agent with tools {string} and {string}")]
fn given_agent_with_tools(world: &mut QuectoWorld, tool1: String, tool2: String) {
    ensure_mock_llm(world);
    world
        .mock_tools
        .insert(tool1.clone(), Arc::new(MockBddTool::new(&tool1, "")));
    world
        .mock_tools
        .insert(tool2.clone(), Arc::new(MockBddTool::new(&tool2, "")));
}

#[given("a fully initialized agent")]
fn given_fully_initialized_agent(world: &mut QuectoWorld) {
    ensure_mock_llm(world);
    // Register some tools to have a non-zero count
    world
        .mock_tools
        .insert("exec".to_string(), Arc::new(MockBddTool::new("exec", "")));
    world.mock_tools.insert(
        "read_file".to_string(),
        Arc::new(MockBddTool::new("read_file", "")),
    );
    world.mock_tools.insert(
        "write_file".to_string(),
        Arc::new(MockBddTool::new("write_file", "")),
    );
}

#[when(expr = "the agent processes message {string}")]
fn when_agent_processes_message(world: &mut QuectoWorld, message: String) {
    let max_iter = world
        .env_overrides
        .get("_max_tool_iterations")
        .and_then(|v| v.parse::<u32>().ok());
    let agent = build_agent_loop(world, max_iter);

    let mut messages = vec![Message {
        role: Role::User,
        content: message,
        tool_calls: vec![],
        tool_call_id: None,
    }];

    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(agent.process(&mut messages));

    world.agent_result = Some(result.expect("agent process failed"));
}

#[when("the agent sends a request to the LLM")]
fn when_agent_sends_request(world: &mut QuectoWorld) {
    let agent = build_agent_loop(world, None);

    // Queue a simple text response so the loop completes
    let mock = world.mock_llm.as_ref().unwrap();
    mock.push_response(LlmResponse {
        content: Some("ok".to_string()),
        tool_calls: vec![],
        usage: None,
    });

    let mut messages = vec![Message {
        role: Role::User,
        content: "test".to_string(),
        tool_calls: vec![],
        tool_call_id: None,
    }];

    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(agent.process(&mut messages))
        .expect("agent process failed while capturing tool definitions");
}

#[when("I query the startup info")]
fn when_query_startup_info(world: &mut QuectoWorld) {
    let agent = build_agent_loop(world, None).with_skill_count(2);
    world.agent_info = Some(agent.info());
}

#[then(expr = "the response should be {string}")]
fn then_response_should_be(world: &mut QuectoWorld, expected: String) {
    let result = world.agent_result.as_ref().expect("no agent result");
    assert_eq!(
        result.response, expected,
        "expected response '{}', got '{}'",
        expected, result.response
    );
}

#[then("both tools should be executed in order")]
fn then_both_tools_executed(world: &mut QuectoWorld) {
    let result = world.agent_result.as_ref().expect("no agent result");
    assert_eq!(
        result.tool_iterations, 2,
        "expected 2 tool iterations, got {}",
        result.tool_iterations
    );
}

#[then("the final response should confirm completion")]
fn then_final_response_confirms_completion(world: &mut QuectoWorld) {
    let result = world.agent_result.as_ref().expect("no agent result");
    assert!(
        !result.response.is_empty(),
        "expected a non-empty final response"
    );
    assert!(
        !result.iteration_limit_reached,
        "should not have hit iteration limit"
    );
}

#[then(expr = "the agent should stop after {int} tool iterations")]
fn then_agent_stops_after_iterations(world: &mut QuectoWorld, expected: u32) {
    let result = world.agent_result.as_ref().expect("no agent result");
    assert_eq!(
        result.tool_iterations, expected,
        "expected {} tool iterations, got {}",
        expected, result.tool_iterations
    );
}

#[then("the response should indicate the iteration limit was reached")]
fn then_response_indicates_limit(world: &mut QuectoWorld) {
    let result = world.agent_result.as_ref().expect("no agent result");
    assert!(
        result.iteration_limit_reached,
        "expected iteration_limit_reached to be true"
    );
    assert!(
        result.response.contains("limit"),
        "expected response to mention 'limit', got: {}",
        result.response
    );
}

#[then(expr = "the request should include tool definitions for {string} and {string}")]
fn then_request_includes_tool_defs(world: &mut QuectoWorld, tool1: String, tool2: String) {
    let mock = world.mock_llm.as_ref().expect("no mock LLM");
    let defs = mock.last_tool_defs();
    let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    assert!(
        names.contains(&tool1.as_str()),
        "expected tool definitions to include '{}', got: {:?}",
        tool1,
        names
    );
    assert!(
        names.contains(&tool2.as_str()),
        "expected tool definitions to include '{}', got: {:?}",
        tool2,
        names
    );
}

#[then("each tool definition should have name, description, and parameters")]
fn then_each_tool_def_has_fields(world: &mut QuectoWorld) {
    let mock = world.mock_llm.as_ref().expect("no mock LLM");
    let defs = mock.last_tool_defs();
    assert!(!defs.is_empty(), "expected at least one tool definition");
    for def in &defs {
        assert!(!def.name.is_empty(), "tool name should not be empty");
        assert!(
            !def.description.is_empty(),
            "tool '{}' description should not be empty",
            def.name
        );
        assert!(
            !def.parameters_schema.is_empty(),
            "tool '{}' parameters_schema should not be empty",
            def.name
        );
    }
}

#[then("it should report the number of loaded tools")]
fn then_report_tool_count(world: &mut QuectoWorld) {
    let info = world.agent_info.as_ref().expect("no agent info");
    assert!(
        info.tool_count > 0,
        "expected tool_count > 0, got {}",
        info.tool_count
    );
}

#[then("it should report the number of available skills")]
fn then_report_skill_count(world: &mut QuectoWorld) {
    let info = world.agent_info.as_ref().expect("no agent info");
    assert!(
        info.skill_count > 0,
        "expected skill_count > 0, got {}",
        info.skill_count
    );
}

// ===========================================================================
// Session Steps
// ===========================================================================

/// Helper: ensure a session workspace with session store.
fn ensure_session_workspace(world: &mut QuectoWorld) {
    if world.session_workspace.is_none() {
        let td = TempDir::new().expect("failed to create temp dir");
        let ws = td.path().to_path_buf();
        world.session_store = Some(FileSessionStore::new(&ws));
        world.memory_store = Some(MemoryStore::new(&ws));
        world.session_workspace = Some(ws);
        world._temp_dir = Some(td);
    }
}

#[given("a session workspace")]
fn given_session_workspace(world: &mut QuectoWorld) {
    ensure_session_workspace(world);
}

#[given(expr = "no session exists for key {string}")]
fn given_no_session_exists(world: &mut QuectoWorld, key: String) {
    let store = world.session_store.as_ref().expect("session store not set");
    let exists = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(store.exists(&key))
        .unwrap();
    assert!(!exists, "session '{}' should not exist yet", key);
}

#[given(expr = "a session {string} with {int} messages in history")]
fn given_session_with_messages(world: &mut QuectoWorld, key: String, count: usize) {
    ensure_session_workspace(world);
    let store = world.session_store.as_ref().expect("session store not set");

    let mut session = Session::new(&key);
    for i in 0..count {
        session.messages.push(Message {
            role: if i % 2 == 0 {
                Role::User
            } else {
                Role::Assistant
            },
            content: format!("Message {}", i + 1),
            tool_calls: vec![],
            tool_call_id: None,
        });
    }

    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(store.save(&session))
        .unwrap();
}

#[given(expr = "a session {string} with messages")]
fn given_session_with_some_messages(world: &mut QuectoWorld, key: String) {
    // Delegate to the parametric version with 2 messages
    given_session_with_messages(world, key, 2);
}

#[given(expr = "the workspace file {string} contains {string}")]
fn given_workspace_file_contains(world: &mut QuectoWorld, filename: String, content: String) {
    let ws = world
        .session_workspace
        .as_ref()
        .expect("session workspace not set");
    let path = ws.join(&filename);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent dirs");
    }
    std::fs::write(&path, &content).expect("write file");
}

#[when(expr = "the session store creates a session for key {string}")]
fn when_create_session(world: &mut QuectoWorld, key: String) {
    let store = world.session_store.as_ref().expect("session store not set");
    let session = Session::new(&key);
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(store.save(&session))
        .unwrap();
}

#[when(expr = "the session store loads session {string}")]
fn when_load_session(world: &mut QuectoWorld, key: String) {
    let store = world.session_store.as_ref().expect("session store not set");
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(store.load(&key))
        .unwrap();
    world.loaded_session = Some(result);
}

#[when("the session is saved to disk")]
fn when_session_saved_to_disk(world: &mut QuectoWorld) {
    // The Given step already persisted the session via store.save().
    // Verify the session directory contains at least one entry, confirming
    // that the save operation produced durable state on disk.
    let ws = world
        .session_workspace
        .as_ref()
        .expect("session workspace not set");
    let sessions_dir = ws.join("sessions");
    let has_files = std::fs::read_dir(&sessions_dir)
        .expect("sessions directory should exist after save")
        .next()
        .is_some();
    assert!(
        has_files,
        "expected at least one session file in {:?}",
        sessions_dir
    );
}

#[when("the session store is recreated from the same directory")]
fn when_session_store_recreated(world: &mut QuectoWorld) {
    let ws = world
        .session_workspace
        .as_ref()
        .expect("session workspace not set")
        .clone();
    world.session_store = Some(FileSessionStore::new(&ws));
}

#[when(expr = "the agent writes a memory note {string}")]
fn when_agent_writes_memory(world: &mut QuectoWorld, note: String) {
    let store = world.memory_store.as_ref().expect("memory store not set");
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(store.append(&note))
        .unwrap();
}

#[when("the agent loads identity from the workspace")]
fn when_agent_loads_identity(world: &mut QuectoWorld) {
    let ws = world
        .session_workspace
        .as_ref()
        .expect("session workspace not set")
        .clone();
    let identity = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(memory_store::load_identity(&ws))
        .unwrap();
    world.loaded_identity = Some(identity);
}

#[when(expr = "user {string} sends a message on channel {string}")]
fn when_user_sends_message_on_channel(world: &mut QuectoWorld, user_id: String, channel: String) {
    let key = Session::build_key(&channel, &user_id);
    // Create or get session for this routing
    let store = world.session_store.as_ref().expect("session store not set");

    let existing = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(store.load(&key))
        .unwrap();

    let session = existing.unwrap_or_else(|| Session::new(&key));
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(store.save(&session))
        .unwrap();

    world.session_keys.insert(user_id, key);
}

#[then(expr = "a session should exist for key {string}")]
fn then_session_exists(world: &mut QuectoWorld, key: String) {
    let store = world.session_store.as_ref().expect("session store not set");
    let exists = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(store.exists(&key))
        .unwrap();
    assert!(exists, "session '{}' should exist", key);
}

#[then("the session should be found")]
fn then_session_found(world: &mut QuectoWorld) {
    let loaded = world
        .loaded_session
        .as_ref()
        .expect("no load was performed");
    assert!(loaded.is_some(), "expected session to be found");
}

#[then(expr = "the conversation history should contain {int} messages")]
fn then_conversation_history_contains(world: &mut QuectoWorld, expected: usize) {
    let loaded = world
        .loaded_session
        .as_ref()
        .expect("no load was performed")
        .as_ref()
        .expect("session should be found");
    assert_eq!(
        loaded.messages.len(),
        expected,
        "expected {} messages in history, got {}",
        expected,
        loaded.messages.len()
    );
}

#[then(expr = "the file {string} should exist in the session workspace")]
fn then_file_exists_in_session_workspace(world: &mut QuectoWorld, filename: String) {
    let ws = world
        .session_workspace
        .as_ref()
        .expect("session workspace not set");
    let path = ws.join(&filename);
    assert!(
        path.exists(),
        "file '{}' should exist at {}",
        filename,
        path.display()
    );
}

#[then(expr = "the memory file should contain {string}")]
fn then_memory_file_contains(world: &mut QuectoWorld, expected: String) {
    let store = world.memory_store.as_ref().expect("memory store not set");
    let content = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(store.read())
        .unwrap();
    assert!(
        content.contains(&expected),
        "expected MEMORY.md to contain '{}', got: {}",
        expected,
        content
    );
}

#[then(expr = "the identity should include {string}")]
fn then_identity_includes(world: &mut QuectoWorld, expected: String) {
    let identity = world.loaded_identity.as_ref().expect("identity not loaded");
    assert!(
        identity.contains(&expected),
        "expected identity to include '{}', got: {}",
        expected,
        identity
    );
}

#[then(expr = "user {string} should have session key {string}")]
fn then_user_has_session_key(world: &mut QuectoWorld, user_id: String, expected_key: String) {
    let key = world
        .session_keys
        .get(&user_id)
        .unwrap_or_else(|| panic!("no session key recorded for user '{}'", user_id));
    assert_eq!(
        key, &expected_key,
        "expected user '{}' to have session key '{}', got '{}'",
        user_id, expected_key, key
    );
}

// ===========================================================================
// Auth Steps
// ===========================================================================

fn ensure_credential_store(world: &mut QuectoWorld) {
    if world.credential_store.is_none() {
        if world._temp_dir.is_none() {
            let td = TempDir::new().expect("failed to create temp dir");
            world._temp_dir = Some(td);
        }
        let base = world._temp_dir.as_ref().unwrap().path().to_path_buf();
        world.credential_store = Some(CredentialStore::new(base));
    }
}

#[given("a credential store")]
fn given_credential_store(world: &mut QuectoWorld) {
    ensure_credential_store(world);
}

#[given("a credential store with no credentials")]
fn given_credential_store_empty(world: &mut QuectoWorld) {
    ensure_credential_store(world);
}

#[given(expr = "a stored credential for {string} with method {string}")]
fn given_stored_credential(world: &mut QuectoWorld, provider: String, method: String) {
    ensure_credential_store(world);
    let store = world
        .credential_store
        .as_ref()
        .expect("credential store not set");
    let auth_method = match method.as_str() {
        "oauth" => AuthMethod::OAuth,
        _ => AuthMethod::Token,
    };
    store
        .store(Credential {
            provider,
            token: "test-token".to_string(),
            method: auth_method,
            expires_at: None,
        })
        .unwrap();
}

#[given(expr = "a stored credential for {string} that is expired")]
fn given_expired_credential(world: &mut QuectoWorld, provider: String) {
    ensure_credential_store(world);
    let store = world
        .credential_store
        .as_ref()
        .expect("credential store not set");
    store
        .store(Credential {
            provider,
            token: "expired-token".to_string(),
            method: AuthMethod::Token,
            expires_at: Some(0), // epoch — always expired
        })
        .unwrap();
}

#[when(expr = "I store a token {string} for provider {string}")]
fn when_store_token(world: &mut QuectoWorld, token: String, provider: String) {
    let store = world
        .credential_store
        .as_ref()
        .expect("credential store not set");
    store
        .store(Credential {
            provider,
            token,
            method: AuthMethod::Token,
            expires_at: None,
        })
        .unwrap();
}

#[when("I check auth status")]
fn when_check_auth_status(world: &mut QuectoWorld) {
    let store = world
        .credential_store
        .as_ref()
        .expect("credential store not set");
    world.auth_status = Some(store.status_summary().unwrap());
}

#[when(expr = "I remove the credential for {string}")]
fn when_remove_credential(world: &mut QuectoWorld, provider: String) {
    let store = world
        .credential_store
        .as_ref()
        .expect("credential store not set");
    store.remove(&provider).unwrap();
}

#[when("I remove all credentials")]
fn when_remove_all_credentials(world: &mut QuectoWorld) {
    let store = world
        .credential_store
        .as_ref()
        .expect("credential store not set");
    store.remove_all().unwrap();
}

#[then(expr = "the credential for {string} should exist")]
fn then_credential_exists(world: &mut QuectoWorld, provider: String) {
    let store = world
        .credential_store
        .as_ref()
        .expect("credential store not set");
    assert!(
        store.exists(&provider).unwrap(),
        "credential for '{}' should exist",
        provider
    );
}

#[then(expr = "the credential for {string} should not exist")]
fn then_credential_not_exists(world: &mut QuectoWorld, provider: String) {
    let store = world
        .credential_store
        .as_ref()
        .expect("credential store not set");
    assert!(
        !store.exists(&provider).unwrap(),
        "credential for '{}' should not exist",
        provider
    );
}

#[then(expr = "the credential token should be {string}")]
fn then_credential_token_is(world: &mut QuectoWorld, expected: String) {
    let store = world
        .credential_store
        .as_ref()
        .expect("credential store not set");
    // Get the most recently stored credential (from the last store operation)
    let list = store.list().unwrap();
    let cred = list.first().expect("no credentials found");
    assert_eq!(
        cred.token, expected,
        "expected token '{}', got '{}'",
        expected, cred.token
    );
}

#[then("the auth status should report no providers")]
fn then_auth_status_no_providers(world: &mut QuectoWorld) {
    let status = world.auth_status.as_ref().expect("no auth status");
    assert!(status.is_empty(), "expected no providers, got {:?}", status);
}

#[then(expr = "the auth status should include {string}")]
fn then_auth_status_includes(world: &mut QuectoWorld, provider: String) {
    let status = world.auth_status.as_ref().expect("no auth status");
    assert!(
        status.iter().any(|s| s.provider == provider),
        "expected auth status to include '{}', got: {:?}",
        provider,
        status.iter().map(|s| &s.provider).collect::<Vec<_>>()
    );
}

#[then(expr = "the auth status for {string} should be {string}")]
fn then_auth_status_for_provider(
    world: &mut QuectoWorld,
    provider: String,
    expected_status: String,
) {
    let status = world.auth_status.as_ref().expect("no auth status");
    let entry = status
        .iter()
        .find(|s| s.provider == provider)
        .unwrap_or_else(|| panic!("no auth status for provider '{}'", provider));
    assert_eq!(
        entry.status, expected_status,
        "expected status '{}' for '{}', got '{}'",
        expected_status, provider, entry.status
    );
}

// ===========================================================================
// Auth CLI Steps
// ===========================================================================

#[given("a quecto base directory at a temporary path")]
fn given_quecto_base_dir_temp(world: &mut QuectoWorld) {
    ensure_temp_dir(world);
}

#[then(expr = "the credential for {string} should exist in the base directory")]
fn then_credential_exists_in_base(world: &mut QuectoWorld, provider: String) {
    let base = base_path(world);
    let store = CredentialStore::new(&base);
    assert!(
        store.exists(&provider).unwrap(),
        "credential for '{}' should exist in base directory {}",
        provider,
        base.display()
    );
}

#[then(expr = "the credential for {string} should not exist in the base directory")]
fn then_credential_not_exists_in_base(world: &mut QuectoWorld, provider: String) {
    let base = base_path(world);
    let store = CredentialStore::new(&base);
    assert!(
        !store.exists(&provider).unwrap(),
        "credential for '{}' should not exist in base directory {}",
        provider,
        base.display()
    );
}

#[given(expr = "a stored credential for {string} in the base directory")]
fn given_stored_credential_in_base(world: &mut QuectoWorld, provider: String) {
    ensure_temp_dir(world);
    let base = base_path(world);
    let store = CredentialStore::new(&base);
    store
        .store(Credential {
            provider,
            token: "test-token".to_string(),
            method: AuthMethod::Token,
            expires_at: None,
        })
        .unwrap();
}

#[given(expr = "a stored credential for {string} with method {string} in the base directory")]
fn given_stored_credential_method_in_base(
    world: &mut QuectoWorld,
    provider: String,
    method: String,
) {
    ensure_temp_dir(world);
    let base = base_path(world);
    let store = CredentialStore::new(&base);
    let auth_method = match method.as_str() {
        "oauth" => AuthMethod::OAuth,
        _ => AuthMethod::Token,
    };
    store
        .store(Credential {
            provider,
            token: "test-token".to_string(),
            method: auth_method,
            expires_at: None,
        })
        .unwrap();
}

#[given(expr = "a stored credential for {string} that is expired in the base directory")]
fn given_expired_credential_in_base(world: &mut QuectoWorld, provider: String) {
    ensure_temp_dir(world);
    let base = base_path(world);
    let store = CredentialStore::new(&base);
    store
        .store(Credential {
            provider,
            token: "expired-token".to_string(),
            method: AuthMethod::Token,
            expires_at: Some(0), // epoch — always expired
        })
        .unwrap();
}

// ===========================================================================
// Auth Gateway Wiring Steps
// ===========================================================================

#[given(expr = "a config with no API key for {string}")]
fn given_config_no_api_key(world: &mut QuectoWorld, _provider: String) {
    ensure_temp_dir(world);
    let base = base_path(world);
    let config: Config = serde_json::from_str("{}").unwrap();
    // Write config to base for gateway to load
    let config_json = serde_json::to_string_pretty(&config).unwrap();
    std::fs::write(base.join("config.json"), config_json).unwrap();
    world.gateway_config = Some(config);
    world.gateway_credential_store = Some(CredentialStore::new(&base));
}

#[given(expr = "a config with API key {string} for {string}")]
fn given_config_with_api_key(world: &mut QuectoWorld, api_key: String, provider: String) {
    ensure_temp_dir(world);
    let base = base_path(world);
    let config_json = match provider.as_str() {
        "openai" => format!(
            r#"{{"providers": {{"openai": {{"api_key": "{}"}}}}}}"#,
            api_key
        ),
        "anthropic" => format!(
            r#"{{"providers": {{"anthropic": {{"api_key": "{}"}}}}}}"#,
            api_key
        ),
        _ => "{}".to_string(),
    };
    let config: Config = serde_json::from_str(&config_json).unwrap();
    std::fs::write(
        base.join("config.json"),
        serde_json::to_string_pretty(&config).unwrap(),
    )
    .unwrap();
    world.gateway_config = Some(config);
    world.gateway_credential_store = Some(CredentialStore::new(&base));
}

#[given(expr = "a stored credential for {string} with token {string}")]
fn given_stored_credential_with_token(world: &mut QuectoWorld, provider: String, token: String) {
    // If gateway_credential_store is set, use it; otherwise use the default credential_store
    if let Some(ref store) = world.gateway_credential_store {
        store
            .store(Credential {
                provider,
                token,
                method: AuthMethod::Token,
                expires_at: None,
            })
            .unwrap();
    } else {
        ensure_credential_store(world);
        let store = world.credential_store.as_ref().unwrap();
        store
            .store(Credential {
                provider,
                token,
                method: AuthMethod::Token,
                expires_at: None,
            })
            .unwrap();
    }
}

#[given(expr = "no stored credential for {string}")]
fn given_no_stored_credential(world: &mut QuectoWorld, provider: String) {
    // Ensure the store exists but has no credential for this provider
    if let Some(ref store) = world.gateway_credential_store {
        let _ = store.remove(&provider);
    }
}

#[when("the gateway initializes providers")]
fn when_gateway_initializes_providers(world: &mut QuectoWorld) {
    use quecto::interface::gateway::resolve_api_key;

    let config = world
        .gateway_config
        .as_ref()
        .expect("gateway config not set");
    let base = base_path(world);
    let store = CredentialStore::new(&base);
    let creds = store.load_snapshot().unwrap_or_default();

    let resolved = resolve_api_key(&config.providers.openai.api_key, &creds, "openai");
    world.gateway_resolved_api_key = Some(resolved);
    world.gateway_cred_snapshot = Some(creds);
}

#[then(expr = "the OpenAI provider should use API key {string}")]
fn then_openai_provider_uses_key(world: &mut QuectoWorld, expected: String) {
    let actual = world
        .gateway_resolved_api_key
        .as_ref()
        .expect("no resolved API key");
    assert_eq!(
        actual, &expected,
        "expected OpenAI API key '{}', got '{}'",
        expected, actual
    );
}

#[when("the gateway checks provider readiness")]
fn when_gateway_checks_readiness(world: &mut QuectoWorld) {
    use quecto::interface::gateway::check_provider_readiness;

    let store = world
        .gateway_credential_store
        .as_ref()
        .or(world.credential_store.as_ref())
        .expect("no credential store set");
    let creds = store.load_snapshot().unwrap_or_default();
    let needs_reauth = check_provider_readiness(&creds);
    world.gateway_readiness_report = Some(needs_reauth);
}

#[then(expr = "the gateway should report {string} needs re-authentication")]
fn then_gateway_reports_reauth(world: &mut QuectoWorld, provider: String) {
    let report = world
        .gateway_readiness_report
        .as_ref()
        .expect("no readiness report");
    assert!(
        report.contains(&provider),
        "expected '{}' to need re-authentication, got: {:?}",
        provider,
        report
    );
}

// ===========================================================================
// Telegram Steps
// ===========================================================================

#[given(expr = "a config with Telegram enabled and token {string}")]
fn given_telegram_enabled(world: &mut QuectoWorld, token: String) {
    world.telegram_config = Some(TelegramConfig {
        enabled: true,
        token,
        allow_from: vec![],
    });
}

#[given("a config with Telegram disabled")]
fn given_telegram_disabled(world: &mut QuectoWorld) {
    world.telegram_config = Some(TelegramConfig {
        enabled: false,
        token: String::new(),
        allow_from: vec![],
    });
}

#[given(expr = "a Telegram channel with allow_from {string}, {string}")]
fn given_telegram_with_allow_from(world: &mut QuectoWorld, user1: String, user2: String) {
    let config = TelegramConfig {
        enabled: true,
        token: "test-token".to_string(),
        allow_from: vec![user1, user2],
    };
    world.telegram_channel = Some(TelegramChannel::new(&config));
}

#[given("a Telegram channel with empty allow_from")]
fn given_telegram_empty_allow_from(world: &mut QuectoWorld) {
    let config = TelegramConfig {
        enabled: true,
        token: "test-token".to_string(),
        allow_from: vec![],
    };
    world.telegram_channel = Some(TelegramChannel::new(&config));
}

#[given(expr = "a raw Telegram update with text {string} from user {string}")]
fn given_raw_telegram_update(world: &mut QuectoWorld, text: String, user_id: String) {
    let uid: i64 = user_id.parse().unwrap();
    world.telegram_update = Some(TelegramUpdate {
        update_id: 1,
        message: Some(TelegramUpdateMessage {
            message_id: 42,
            from: Some(TelegramUser {
                id: uid,
                first_name: Some("Test".to_string()),
                username: None,
            }),
            chat: TelegramChat {
                id: uid,
                chat_type: Some("private".to_string()),
            },
            text: Some(text),
        }),
    });
}

#[when("the Telegram channel is created")]
fn when_telegram_created(world: &mut QuectoWorld) {
    let config = world
        .telegram_config
        .as_ref()
        .expect("telegram config not set");
    world.telegram_channel = Some(TelegramChannel::new(config));
}

#[when("I check if Telegram is enabled")]
fn when_check_telegram_enabled(world: &mut QuectoWorld) {
    // Evaluate the enabled flag from config without constructing a full
    // TelegramChannel (which allocates a reqwest::Client).
    let config = world
        .telegram_config
        .as_ref()
        .expect("telegram config not set");
    let enabled = config.enabled && !config.token.is_empty();
    world.telegram_enabled_check = Some(enabled);
}

#[when(expr = "user {string} sends a message")]
fn when_user_sends_telegram_message(world: &mut QuectoWorld, user_id: String) {
    let ch = world
        .telegram_channel
        .as_ref()
        .expect("telegram channel not set");
    world.telegram_filter_result = Some(ch.is_user_allowed(&user_id));
}

#[when("the update is parsed")]
fn when_update_parsed(world: &mut QuectoWorld) {
    let update = world
        .telegram_update
        .as_ref()
        .expect("telegram update not set");
    world.telegram_parsed_message = TelegramChannel::parse_update(update);
}

#[then(expr = "the channel name should be {string}")]
fn then_channel_name(world: &mut QuectoWorld, expected: String) {
    let ch = world
        .telegram_channel
        .as_ref()
        .expect("telegram channel not set");
    assert_eq!(ch.name(), expected);
}

#[then("the channel should be enabled")]
fn then_channel_enabled(world: &mut QuectoWorld) {
    let ch = world
        .telegram_channel
        .as_ref()
        .expect("telegram channel not set");
    assert!(ch.is_enabled(), "channel should be enabled");
}

#[then("the Telegram channel should not be enabled")]
fn then_telegram_not_enabled(world: &mut QuectoWorld) {
    // Prefer the lightweight enabled-check result (set by "When I check if
    // Telegram is enabled") over the full channel object.
    if let Some(enabled) = world.telegram_enabled_check {
        assert!(!enabled, "channel should not be enabled");
    } else {
        let ch = world
            .telegram_channel
            .as_ref()
            .expect("telegram channel or enabled check not set");
        assert!(!ch.is_enabled(), "channel should not be enabled");
    }
}

#[then("the message should pass the allow_from filter")]
fn then_message_passes_filter(world: &mut QuectoWorld) {
    let result = world.telegram_filter_result.expect("no filter result");
    assert!(result, "message should pass the allow_from filter");
}

#[then("the message should be rejected by the allow_from filter")]
fn then_message_rejected_by_filter(world: &mut QuectoWorld) {
    let result = world.telegram_filter_result.expect("no filter result");
    assert!(
        !result,
        "message should be rejected by the allow_from filter"
    );
}

#[then(expr = "the parsed message text should be {string}")]
fn then_parsed_text(world: &mut QuectoWorld, expected: String) {
    let msg = world
        .telegram_parsed_message
        .as_ref()
        .expect("no parsed message");
    assert_eq!(msg.text, expected);
}

#[then(expr = "the parsed sender ID should be {string}")]
fn then_parsed_sender_id(world: &mut QuectoWorld, expected: String) {
    let msg = world
        .telegram_parsed_message
        .as_ref()
        .expect("no parsed message");
    assert_eq!(msg.sender_id, expected);
}

// ===========================================================================
// Cron Steps
// ===========================================================================

fn ensure_cron_store(world: &mut QuectoWorld) {
    if world.cron_store.is_none() {
        if world._temp_dir.is_none() {
            let td = TempDir::new().expect("failed to create temp dir");
            world._temp_dir = Some(td);
        }
        let base = world._temp_dir.as_ref().unwrap().path().to_path_buf();
        world.cron_workspace = Some(base.clone());
        world.cron_store = Some(FileCronStore::new(base));
    }
}

fn make_interval_job(name: &str, seconds: u64) -> CronJob {
    CronJob {
        id: name.to_lowercase().replace(' ', "-"),
        name: name.to_string(),
        message: format!("Run {}", name),
        schedule: CronSchedule::Interval { seconds },
        enabled: true,
        deliver_to: None,
    }
}

fn make_cron_expr_job(name: &str, expr: &str) -> CronJob {
    CronJob {
        id: name.to_lowercase().replace(' ', "-"),
        name: name.to_string(),
        message: format!("Run {}", name),
        schedule: CronSchedule::Cron {
            expression: expr.to_string(),
        },
        enabled: true,
        deliver_to: None,
    }
}

#[given("a cron store")]
fn given_cron_store(world: &mut QuectoWorld) {
    ensure_cron_store(world);
}

#[given(expr = "a job {string} with interval {int} seconds exists")]
fn given_job_with_interval(world: &mut QuectoWorld, name: String, seconds: u64) {
    ensure_cron_store(world);
    let store = world.cron_store.as_ref().unwrap();
    store.add(make_interval_job(&name, seconds)).unwrap();
}

#[given(expr = "a job {string} with cron expression {string} exists")]
fn given_job_with_cron_expr(world: &mut QuectoWorld, name: String, expr: String) {
    ensure_cron_store(world);
    let store = world.cron_store.as_ref().unwrap();
    store.add(make_cron_expr_job(&name, &expr)).unwrap();
}

#[given(expr = "a disabled job {string} with interval {int} seconds exists")]
fn given_disabled_job(world: &mut QuectoWorld, name: String, seconds: u64) {
    ensure_cron_store(world);
    let store = world.cron_store.as_ref().unwrap();
    let mut job = make_interval_job(&name, seconds);
    job.enabled = false;
    store.add(job).unwrap();
}

#[when(expr = "I add a job {string} with interval {int} seconds")]
fn when_add_interval_job(world: &mut QuectoWorld, name: String, seconds: u64) {
    let store = world.cron_store.as_ref().unwrap();
    store.add(make_interval_job(&name, seconds)).unwrap();
}

#[when(expr = "I add a job {string} with cron expression {string}")]
fn when_add_cron_expr_job(world: &mut QuectoWorld, name: String, expr: String) {
    let store = world.cron_store.as_ref().unwrap();
    store.add(make_cron_expr_job(&name, &expr)).unwrap();
}

#[when("I list all jobs")]
fn when_list_jobs(world: &mut QuectoWorld) {
    let store = world.cron_store.as_ref().unwrap();
    world.cron_jobs = Some(store.list().unwrap());
}

#[when(expr = "I remove the job {string}")]
fn when_remove_job(world: &mut QuectoWorld, name: String) {
    let store = world.cron_store.as_ref().unwrap();
    let job = cron_store::find_by_name(store, &name)
        .unwrap()
        .unwrap_or_else(|| panic!("job '{}' not found", name));
    store.remove(&job.id).unwrap();
}

#[when(expr = "I disable the job {string}")]
fn when_disable_job(world: &mut QuectoWorld, name: String) {
    let store = world.cron_store.as_ref().unwrap();
    let job = cron_store::find_by_name(store, &name)
        .unwrap()
        .unwrap_or_else(|| panic!("job '{}' not found", name));
    store.set_enabled(&job.id, false).unwrap();
}

#[when(expr = "I enable the job {string}")]
fn when_enable_job(world: &mut QuectoWorld, name: String) {
    let store = world.cron_store.as_ref().unwrap();
    let job = cron_store::find_by_name(store, &name)
        .unwrap()
        .unwrap_or_else(|| panic!("job '{}' not found", name));
    store.set_enabled(&job.id, true).unwrap();
}

#[when("the cron store is recreated from the same directory")]
fn when_cron_store_recreated(world: &mut QuectoWorld) {
    let ws = world.cron_workspace.as_ref().unwrap().clone();
    world.cron_store = Some(FileCronStore::new(ws));
}

#[then(expr = "the job {string} should exist in the store")]
fn then_job_exists(world: &mut QuectoWorld, name: String) {
    let store = world.cron_store.as_ref().unwrap();
    let found = cron_store::find_by_name(store, &name).unwrap();
    assert!(found.is_some(), "job '{}' should exist", name);
}

#[then(expr = "the job {string} should not exist in the store")]
fn then_job_not_exists(world: &mut QuectoWorld, name: String) {
    let store = world.cron_store.as_ref().unwrap();
    let found = cron_store::find_by_name(store, &name).unwrap();
    assert!(found.is_none(), "job '{}' should not exist", name);
}

#[then("the job should be enabled")]
fn then_job_enabled(world: &mut QuectoWorld) {
    let store = world.cron_store.as_ref().unwrap();
    let jobs = store.list().unwrap();
    let last = jobs.last().expect("no jobs");
    assert!(last.enabled, "job should be enabled");
}

#[then(expr = "the job {string} should be disabled")]
fn then_job_disabled(world: &mut QuectoWorld, name: String) {
    let store = world.cron_store.as_ref().unwrap();
    let job = cron_store::find_by_name(store, &name).unwrap().unwrap();
    assert!(!job.enabled, "job '{}' should be disabled", name);
}

#[then(expr = "the job {string} should be enabled")]
fn then_named_job_enabled(world: &mut QuectoWorld, name: String) {
    let store = world.cron_store.as_ref().unwrap();
    let job = cron_store::find_by_name(store, &name).unwrap().unwrap();
    assert!(job.enabled, "job '{}' should be enabled", name);
}

#[then(expr = "the job list should contain {int} jobs")]
fn then_job_list_count(world: &mut QuectoWorld, expected: usize) {
    let jobs = world.cron_jobs.as_ref().expect("no job list");
    assert_eq!(
        jobs.len(),
        expected,
        "expected {} jobs, got {}",
        expected,
        jobs.len()
    );
}

#[then(expr = "the job list should include {string}")]
fn then_job_list_includes(world: &mut QuectoWorld, name: String) {
    let jobs = world.cron_jobs.as_ref().expect("no job list");
    assert!(
        jobs.iter().any(|j| j.name == name),
        "job list should include '{}', has: {:?}",
        name,
        jobs.iter().map(|j| &j.name).collect::<Vec<_>>()
    );
}

// ===========================================================================
// Skills Steps
// ===========================================================================

/// Helper: ensure skill loader temp dirs exist.
fn ensure_skill_dirs(world: &mut QuectoWorld) {
    if world.skill_loader_workspace.is_none() {
        let ws = TempDir::new().expect("temp dir");
        let global = TempDir::new().expect("temp dir");
        let builtin = TempDir::new().expect("temp dir");
        world.skill_loader_workspace = Some(ws.path().to_path_buf());
        world.skill_loader_global = Some(global.path().to_path_buf());
        world.skill_loader_builtin = Some(builtin.path().to_path_buf());
        world._skill_temp_dirs.push(ws);
        world._skill_temp_dirs.push(global);
        world._skill_temp_dirs.push(builtin);
    }
}

fn create_workspace_skill(base: &Path, name: &str, content: Option<&str>) {
    let skill_dir = base.join("skills").join(name);
    std::fs::create_dir_all(&skill_dir).expect("create skill dir");
    if let Some(c) = content {
        std::fs::write(skill_dir.join("SKILL.md"), c).expect("write SKILL.md");
    }
}

fn create_global_skill(base: &Path, name: &str, content: &str) {
    let skill_dir = base.join("skills").join(name);
    std::fs::create_dir_all(&skill_dir).expect("create skill dir");
    std::fs::write(skill_dir.join("SKILL.md"), content).expect("write SKILL.md");
}

fn create_builtin_skill_dir(base: &Path, name: &str, content: &str) {
    let skill_dir = base.join(name);
    std::fs::create_dir_all(&skill_dir).expect("create skill dir");
    std::fs::write(skill_dir.join("SKILL.md"), content).expect("write SKILL.md");
}

fn build_skill_loader(world: &QuectoWorld) -> FileSkillLoader {
    FileSkillLoader::new(
        world.skill_loader_workspace.as_ref().expect("ws"),
        world.skill_loader_global.as_ref().expect("global"),
        world.skill_loader_builtin.as_ref().expect("builtin"),
    )
}

#[given(expr = "a workspace with skill {string} installed")]
fn given_workspace_skill_installed(world: &mut QuectoWorld, name: String) {
    ensure_temp_dir(world);
    let skill_dir = base_path(world)
        .join("workspace")
        .join("skills")
        .join(&name);
    std::fs::create_dir_all(&skill_dir).expect("create skill dir");
    std::fs::write(skill_dir.join("SKILL.md"), format!("{} skill", name)).expect("write SKILL.md");
}

#[given(expr = "a skill loader with workspace skill {string} containing {string}")]
fn given_workspace_skill(world: &mut QuectoWorld, name: String, content: String) {
    ensure_skill_dirs(world);
    create_workspace_skill(
        world.skill_loader_workspace.as_ref().unwrap(),
        &name,
        Some(&content),
    );
}

#[given(expr = "a skill loader with global skill {string} containing {string}")]
fn given_global_skill(world: &mut QuectoWorld, name: String, content: String) {
    ensure_skill_dirs(world);
    create_global_skill(world.skill_loader_global.as_ref().unwrap(), &name, &content);
}

#[given(expr = "a skill loader with builtin skill {string} containing {string}")]
fn given_builtin_skill(world: &mut QuectoWorld, name: String, content: String) {
    ensure_skill_dirs(world);
    create_builtin_skill_dir(
        world.skill_loader_builtin.as_ref().unwrap(),
        &name,
        &content,
    );
}

#[given("an empty skill loader")]
fn given_empty_skill_loader(world: &mut QuectoWorld) {
    ensure_skill_dirs(world);
}

#[given(expr = "a skill loader with workspace skill {string} without SKILL.md")]
fn given_workspace_skill_no_md(world: &mut QuectoWorld, name: String) {
    ensure_skill_dirs(world);
    create_workspace_skill(world.skill_loader_workspace.as_ref().unwrap(), &name, None);
}

#[when("the skills loader lists all skills")]
fn when_skills_list(world: &mut QuectoWorld) {
    let loader = build_skill_loader(world);
    world.skill_list = Some(loader.list().unwrap());
}

#[when(expr = "the skill {string} is loaded by name")]
fn when_skill_loaded_by_name(world: &mut QuectoWorld, name: String) {
    let loader = build_skill_loader(world);
    world.loaded_skill = Some(loader.load(&name).unwrap());
}

#[then(expr = "the skill list should contain {int} skill")]
fn then_skill_list_count_singular(world: &mut QuectoWorld, expected: usize) {
    let skills = world.skill_list.as_ref().expect("no skill list");
    assert_eq!(
        skills.len(),
        expected,
        "expected {} skills, got {}",
        expected,
        skills.len()
    );
}

#[then(expr = "the skill list should contain {int} skills")]
fn then_skill_list_count(world: &mut QuectoWorld, expected: usize) {
    let skills = world.skill_list.as_ref().expect("no skill list");
    assert_eq!(
        skills.len(),
        expected,
        "expected {} skills, got {}",
        expected,
        skills.len()
    );
}

#[then(expr = "the skill list should include {string}")]
fn then_skill_list_includes(world: &mut QuectoWorld, name: String) {
    let skills = world.skill_list.as_ref().expect("no skill list");
    assert!(
        skills.iter().any(|s| s.name == name),
        "skill list should include '{}', has: {:?}",
        name,
        skills.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
}

#[then(expr = "the skill {string} should have source {string}")]
fn then_skill_has_source(world: &mut QuectoWorld, name: String, expected_source: String) {
    let skills = world.skill_list.as_ref().expect("no skill list");
    let skill = skills
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("skill '{}' not found in list", name));
    let source_str = match skill.source {
        SkillSource::Workspace => "workspace",
        SkillSource::Global => "global",
        SkillSource::Builtin => "builtin",
    };
    assert_eq!(
        source_str, expected_source,
        "expected skill '{}' source '{}', got '{}'",
        name, expected_source, source_str
    );
}

#[then("the loaded skill should exist")]
fn then_loaded_skill_exists(world: &mut QuectoWorld) {
    let loaded = world.loaded_skill.as_ref().expect("no load was performed");
    assert!(loaded.is_some(), "expected skill to be found");
}

#[then("the loaded skill should not exist")]
fn then_loaded_skill_not_exists(world: &mut QuectoWorld) {
    let loaded = world.loaded_skill.as_ref().expect("no load was performed");
    assert!(loaded.is_none(), "expected skill to not be found");
}

#[then(expr = "the loaded skill content should contain {string}")]
fn then_loaded_skill_content(world: &mut QuectoWorld, expected: String) {
    let loaded = world
        .loaded_skill
        .as_ref()
        .expect("no load was performed")
        .as_ref()
        .expect("skill should be found");
    assert!(
        loaded.content.contains(&expected),
        "expected skill content to contain '{}', got: {}",
        expected,
        loaded.content
    );
}

#[then(expr = "the loaded skill should have source {string}")]
fn then_loaded_skill_source(world: &mut QuectoWorld, expected_source: String) {
    let loaded = world
        .loaded_skill
        .as_ref()
        .expect("no load was performed")
        .as_ref()
        .expect("skill should be found");
    let source_str = match loaded.source {
        SkillSource::Workspace => "workspace",
        SkillSource::Global => "global",
        SkillSource::Builtin => "builtin",
    };
    assert_eq!(
        source_str, expected_source,
        "expected source '{}', got '{}'",
        expected_source, source_str
    );
}

#[then(expr = "the skill {string} should have empty content")]
fn then_skill_empty_content(world: &mut QuectoWorld, name: String) {
    let skills = world.skill_list.as_ref().expect("no skill list");
    let skill = skills
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("skill '{}' not found", name));
    assert!(
        skill.content.is_empty(),
        "expected skill '{}' to have empty content, got: {}",
        name,
        skill.content
    );
}

// ===========================================================================
// Heartbeat Steps
// ===========================================================================

#[given(expr = "a HEARTBEAT.md with content:")]
fn given_heartbeat_content(world: &mut QuectoWorld, step: &gherkin::Step) {
    let content = step.docstring().expect("step should have a docstring");
    world.heartbeat_content = Some(content.to_string());
}

#[given(expr = "a workspace with a HEARTBEAT.md file containing:")]
fn given_workspace_with_heartbeat(world: &mut QuectoWorld, step: &gherkin::Step) {
    let content = step.docstring().expect("step should have a docstring");
    let td = TempDir::new().expect("failed to create temp dir");
    let ws = td.path().to_path_buf();
    std::fs::write(ws.join("HEARTBEAT.md"), content).expect("write HEARTBEAT.md");
    world.heartbeat_workspace = Some(ws);
    world._temp_dir = Some(td);
}

#[given("a workspace without a HEARTBEAT.md file")]
fn given_workspace_without_heartbeat(world: &mut QuectoWorld) {
    let td = TempDir::new().expect("failed to create temp dir");
    world.heartbeat_workspace = Some(td.path().to_path_buf());
    world._temp_dir = Some(td);
}

#[given(expr = "a heartbeat result with {int} tasks found, {int} executed, and ok {word}")]
fn given_heartbeat_result(world: &mut QuectoWorld, found: usize, executed: usize, ok: String) {
    world.heartbeat_result = Some(HeartbeatResult {
        tasks_found: found,
        tasks_executed: executed,
        ok: ok == "true",
    });
}

#[when("the heartbeat content is parsed")]
fn when_heartbeat_parsed(world: &mut QuectoWorld) {
    let content = world
        .heartbeat_content
        .as_ref()
        .expect("heartbeat content not set");
    world.heartbeat_tasks = Some(heartbeat::parse_heartbeat(content));
}

#[when("the heartbeat loads tasks from the workspace")]
fn when_heartbeat_loads_tasks(world: &mut QuectoWorld) {
    let ws = world
        .heartbeat_workspace
        .as_ref()
        .expect("heartbeat workspace not set")
        .clone();
    let tasks = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(heartbeat::load_tasks(&ws))
        .unwrap();
    world.heartbeat_tasks = Some(tasks);
}

#[then(expr = "the parsed tasks should contain {int} items")]
fn then_parsed_tasks_count(world: &mut QuectoWorld, expected: usize) {
    let tasks = world.heartbeat_tasks.as_ref().expect("no parsed tasks");
    assert_eq!(
        tasks.len(),
        expected,
        "expected {} tasks, got {}",
        expected,
        tasks.len()
    );
}

#[then(expr = "task {int} should be {string}")]
fn then_task_message(world: &mut QuectoWorld, index: usize, expected: String) {
    let tasks = world.heartbeat_tasks.as_ref().expect("no parsed tasks");
    let task = &tasks[index - 1]; // 1-indexed
    assert_eq!(
        task.message, expected,
        "expected task {} to be '{}', got '{}'",
        index, expected, task.message
    );
}

#[then("no tasks should be marked as spawn")]
fn then_no_spawn_tasks(world: &mut QuectoWorld) {
    let tasks = world.heartbeat_tasks.as_ref().expect("no parsed tasks");
    assert!(
        tasks.iter().all(|t| !t.use_spawn),
        "expected no spawn tasks, but some are marked as spawn"
    );
}

#[then(expr = "task {int} should be marked as spawn")]
fn then_task_is_spawn(world: &mut QuectoWorld, index: usize) {
    let tasks = world.heartbeat_tasks.as_ref().expect("no parsed tasks");
    let task = &tasks[index - 1];
    assert!(
        task.use_spawn,
        "expected task {} to be marked as spawn",
        index
    );
}

#[then(expr = "task {int} should not be marked as spawn")]
fn then_task_not_spawn(world: &mut QuectoWorld, index: usize) {
    let tasks = world.heartbeat_tasks.as_ref().expect("no parsed tasks");
    let task = &tasks[index - 1];
    assert!(
        !task.use_spawn,
        "expected task {} to NOT be marked as spawn",
        index
    );
}

#[then(expr = "the heartbeat status should be {string}")]
fn then_heartbeat_status(world: &mut QuectoWorld, expected: String) {
    let result = world
        .heartbeat_result
        .as_ref()
        .expect("no heartbeat result");
    assert_eq!(
        result.status(),
        expected,
        "expected status '{}', got '{}'",
        expected,
        result.status()
    );
}

// ===========================================================================
// Subagent Steps
// ===========================================================================

#[given(expr = "a subagent spawn request with task {string}")]
fn given_subagent_spawn_request(world: &mut QuectoWorld, task: String) {
    world.subagent_config = Some(SubagentConfig {
        task,
        agent_id: None,
        restrict_to_workspace: false,
        deliver_to: None,
    });
}

#[given(expr = "a parent agent config with restrict_to_workspace {word}")]
fn given_parent_config_restrict(world: &mut QuectoWorld, value: String) {
    let restrict = value == "true";
    world.subagent_config = Some(SubagentConfig {
        task: "test task".to_string(),
        agent_id: None,
        restrict_to_workspace: restrict,
        deliver_to: None,
    });
}

#[given(expr = "an agent allowlist containing {string} and {string}")]
fn given_agent_allowlist(world: &mut QuectoWorld, agent1: String, agent2: String) {
    world.agent_allowlist = vec![agent1, agent2];
}

#[when("the subagent context is created")]
fn when_subagent_context_created(world: &mut QuectoWorld) {
    let config = world
        .subagent_config
        .as_ref()
        .expect("subagent config not set");
    world.subagent_context = Some(SubagentContext::from_config(config));
}

#[when("a subagent context is created from the parent")]
fn when_subagent_context_from_parent(world: &mut QuectoWorld) {
    let config = world
        .subagent_config
        .as_ref()
        .expect("subagent config not set");
    world.subagent_context = Some(SubagentContext::from_config(config));
}

#[when(expr = "I validate agent_id {string}")]
fn when_validate_agent_id(world: &mut QuectoWorld, agent_id: String) {
    let result = validate_agent_id(&agent_id, &world.agent_allowlist);
    world.agent_id_validation = Some(result.map_err(|e| e.to_string()));
}

#[then(expr = "the subagent context should have task {string}")]
fn then_subagent_has_task(world: &mut QuectoWorld, expected: String) {
    let ctx = world
        .subagent_context
        .as_ref()
        .expect("subagent context not created");
    assert_eq!(
        ctx.task, expected,
        "expected task '{}', got '{}'",
        expected, ctx.task
    );
}

#[then("the subagent context should have an empty conversation history")]
fn then_subagent_empty_history(world: &mut QuectoWorld) {
    let ctx = world
        .subagent_context
        .as_ref()
        .expect("subagent context not created");
    assert!(
        ctx.messages.is_empty(),
        "expected empty conversation history, got {} messages",
        ctx.messages.len()
    );
}

#[then(expr = "the subagent should also have restrict_to_workspace {word}")]
fn then_subagent_restrict(world: &mut QuectoWorld, expected: String) {
    let ctx = world
        .subagent_context
        .as_ref()
        .expect("subagent context not created");
    let expected_bool = expected == "true";
    assert_eq!(
        ctx.restrict_to_workspace, expected_bool,
        "expected restrict_to_workspace {}, got {}",
        expected_bool, ctx.restrict_to_workspace
    );
}

#[then("the validation should succeed")]
fn then_validation_succeeds(world: &mut QuectoWorld) {
    let result = world
        .agent_id_validation
        .as_ref()
        .expect("no validation result");
    assert!(
        result.is_ok(),
        "expected validation to succeed, got: {}",
        result.as_ref().unwrap_err()
    );
}

#[then(expr = "the validation should fail with {string}")]
fn then_validation_fails_with(world: &mut QuectoWorld, expected: String) {
    let result = world
        .agent_id_validation
        .as_ref()
        .expect("no validation result");
    assert!(result.is_err(), "expected validation to fail");
    let err = result.as_ref().unwrap_err();
    assert!(
        err.contains(&expected),
        "expected error to contain '{}', got: {}",
        expected,
        err
    );
}

// ===========================================================================
// Subagent + Message Tool Steps
// ===========================================================================

#[given(expr = "a subagent with deliver_to {string}")]
fn given_subagent_with_deliver_to(world: &mut QuectoWorld, deliver_to: String) {
    world.subagent_config = Some(SubagentConfig {
        task: "test task".to_string(),
        agent_id: None,
        restrict_to_workspace: false,
        deliver_to: Some(deliver_to),
    });
    world.subagent_context = Some(SubagentContext::from_config(
        world.subagent_config.as_ref().unwrap(),
    ));
}

#[given("a message tool connected to the bus")]
fn given_message_tool_on_bus(world: &mut QuectoWorld) {
    let deliver_to = world
        .subagent_context
        .as_ref()
        .expect("subagent context not set")
        .deliver_to
        .clone();

    let mut bus = MessageBus::new(16);
    let sender = bus.outbound_sender();
    let receiver = bus.take_outbound_receiver().unwrap();
    world.message_bus_receiver = Some(receiver);

    let tool = MessageTool::new(sender, deliver_to);
    let mut registry = ToolRegistryImpl::new();
    registry.register(Arc::new(tool));
    world.tool_registry = Some(registry);
}

#[when(expr = "the subagent sends result {string} via the message tool")]
fn when_subagent_sends_via_message(world: &mut QuectoWorld, text: String) {
    let registry = world.tool_registry.as_ref().expect("tool registry not set");
    let args = serde_json::json!({"text": text}).to_string();
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(registry.execute("message", &args))
        .unwrap();
    assert!(!result.is_error, "message send failed: {}", result.content);
}

// ===========================================================================
// Voice Steps
// ===========================================================================

#[given(expr = "a Groq Whisper client with api_key {string}")]
fn given_whisper_client_with_key(world: &mut QuectoWorld, api_key: String) {
    // Client will be reconfigured once the mock server is set up
    world.whisper_client = Some(GroqWhisperClient::new(&api_key));
}

#[given("a Groq Whisper client with no api_key")]
fn given_whisper_client_no_key(world: &mut QuectoWorld) {
    world.whisper_client = Some(GroqWhisperClient::new(""));
}

#[given(expr = "a mock Whisper API that returns transcription {string}")]
fn given_mock_whisper_success(world: &mut QuectoWorld, text: String) {
    // Use a single tokio runtime for mock server setup + keep it alive
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (uri, server) = rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/openai/v1/audio/transcriptions"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"text": text})),
            )
            .mount(&server)
            .await;
        let uri = server.uri();
        (uri, server)
    });

    world.whisper_client = Some(GroqWhisperClient::with_base_url("gsk-test-key", &uri));
    world._wiremock_server_uri = Some(uri);
    // Leak both the runtime and server so the mock HTTP server stays alive
    std::mem::forget(server);
    std::mem::forget(rt);
}

#[given("a mock Whisper API that returns an error")]
fn given_mock_whisper_error(world: &mut QuectoWorld) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (uri, server) = rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/openai/v1/audio/transcriptions"))
            .respond_with(
                wiremock::ResponseTemplate::new(500).set_body_string("Internal Server Error"),
            )
            .mount(&server)
            .await;
        let uri = server.uri();
        (uri, server)
    });

    world.whisper_client = Some(GroqWhisperClient::with_base_url("gsk-test-key", &uri));
    world._wiremock_server_uri = Some(uri);
    std::mem::forget(server);
    std::mem::forget(rt);
}

#[when("the whisper client transcribes audio")]
fn when_client_transcribes(world: &mut QuectoWorld) {
    let client = world
        .whisper_client
        .as_ref()
        .expect("whisper client not set");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result =
        rt.block_on(client.transcribe_bytes(b"fake audio data".to_vec(), "test_audio.ogg"));

    world.transcription_result = Some(result.map_err(|e| e.to_string()));
}

#[then(expr = "the transcription result should be {string}")]
fn then_transcription_is(world: &mut QuectoWorld, expected: String) {
    let result = world
        .transcription_result
        .as_ref()
        .expect("no transcription result");
    match result {
        Ok(tr) => assert_eq!(
            tr.text, expected,
            "expected transcription '{}', got '{}'",
            expected, tr.text
        ),
        Err(e) => panic!("expected successful transcription, got error: {}", e),
    }
}

#[then(expr = "the transcription should fail with {string}")]
fn then_transcription_fails_with(world: &mut QuectoWorld, expected_msg: String) {
    let result = world
        .transcription_result
        .as_ref()
        .expect("no transcription result");
    match result {
        Ok(tr) => panic!(
            "expected transcription to fail with '{}', but got success: '{}'",
            expected_msg, tr.text
        ),
        Err(e) => assert!(
            e.contains(&expected_msg),
            "expected error containing '{}', got: {}",
            expected_msg,
            e
        ),
    }
}

#[then("the transcription should fail with an error message")]
fn then_transcription_fails_with_any(world: &mut QuectoWorld) {
    let result = world
        .transcription_result
        .as_ref()
        .expect("no transcription result");
    assert!(
        result.is_err(),
        "expected transcription to fail, but got success: {:?}",
        result
    );
}

// ===========================================================================
// Observability Steps
// ===========================================================================

#[given("a valid config with OpenAI API key set")]
fn given_valid_config_with_openai(world: &mut QuectoWorld) {
    ensure_temp_dir(world);
    let config_json = r#"{
        "providers": {
            "openai": { "api_key": "sk-test-key-123" }
        }
    }"#;
    let config_path = base_path(world).join("config.json");
    std::fs::write(&config_path, config_json).expect("write config");
}

#[given("a config with OpenAI api_key set and Anthropic not set")]
fn given_config_openai_set_anthropic_not(world: &mut QuectoWorld) {
    ensure_temp_dir(world);
    let config_json = r#"{
        "providers": {
            "openai": { "api_key": "sk-test-key-456" },
            "anthropic": { "api_key": "" }
        }
    }"#;
    let config_path = base_path(world).join("config.json");
    std::fs::write(&config_path, config_json).expect("write config");
}

#[given(expr = "a config with OpenAI api_key {string} set")]
fn given_config_with_specific_openai_key(world: &mut QuectoWorld, api_key: String) {
    ensure_temp_dir(world);
    let config_json = format!(
        r#"{{
        "providers": {{
            "openai": {{ "api_key": "{}" }}
        }}
    }}"#,
        api_key
    );
    let config_path = base_path(world).join("config.json");
    std::fs::write(&config_path, config_json).expect("write config");
}

#[then(expr = "the output should not contain {string}")]
fn then_output_should_not_contain(world: &mut QuectoWorld, unexpected: String) {
    let combined = format!("{}{}", world.stdout, world.stderr);
    assert!(
        !combined.contains(&unexpected),
        "expected output NOT to contain '{}', but got:\nstdout: {}\nstderr: {}",
        unexpected,
        world.stdout,
        world.stderr
    );
}

// ===========================================================================
// Agent CLI — Headless One-Shot Mode Steps
// ===========================================================================

#[given("a temp base directory")]
fn given_temp_base_directory(world: &mut QuectoWorld) {
    let td = TempDir::new().expect("failed to create temp dir");
    world.cli_context.base_dir = Some(td.path().to_path_buf());
    world._temp_dir = Some(td);
}

#[given("a config file with an OpenAI provider pointing at a mock server")]
fn given_config_with_openai_mock(world: &mut QuectoWorld) {
    // Start a wiremock server and leak it so it stays alive for the scenario.
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(wiremock::MockServer::start());
    let uri = server.uri();
    world._wiremock_server_uri = Some(uri.clone());

    // Write config with api_base pointing at the mock server.
    ensure_temp_dir(world);
    let base = base_path(world);
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace dir");
    let config_json = format!(
        r#"{{
  "providers": {{
    "openai": {{ "api_key": "sk-test-key", "api_base": "{uri}" }}
  }},
  "agents": {{
    "defaults": {{
      "workspace": "{workspace}"
    }}
  }}
}}"#,
        uri = uri,
        workspace = workspace.display()
    );
    std::fs::write(base.join("config.json"), config_json).expect("write config");

    std::mem::forget(server);
    std::mem::forget(rt);
}

#[given(expr = "the mock LLM returns a text response {string}")]
fn given_mock_llm_text_response(world: &mut QuectoWorld, content: String) {
    // Verify a mock server was previously configured (from the config step).
    assert!(
        world._wiremock_server_uri.is_some(),
        "mock server URI not set — ensure a config step ran first"
    );
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        // wiremock doesn't support reconnecting to an existing server.
        // Start a new server, mount the mock, and rewrite the config to point at it.
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();
        let response_body = serde_json::json!({
            "id": "chatcmpl-test",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": content
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5,
                "total_tokens": 15
            }
        });
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(response_body))
            .mount(&server)
            .await;

        // Update the config to point at this new server.
        let base = base_path(world);
        let workspace = base.join("workspace");
        let config_json = format!(
            r#"{{
  "providers": {{
    "openai": {{ "api_key": "sk-test-key", "api_base": "{new_uri}" }}
  }},
  "agents": {{
    "defaults": {{
      "workspace": "{workspace}"
    }}
  }}
}}"#,
            new_uri = new_uri,
            workspace = workspace.display()
        );
        std::fs::write(base.join("config.json"), config_json).expect("rewrite config");
        world._wiremock_server_uri = Some(new_uri);
        std::mem::forget(server);
    });
    std::mem::forget(rt);
}

#[given("the mock LLM returns an HTTP 500 error")]
fn given_mock_llm_500_error(world: &mut QuectoWorld) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(
                wiremock::ResponseTemplate::new(500).set_body_string("Internal Server Error"),
            )
            .mount(&server)
            .await;

        let base = base_path(world);
        let workspace = base.join("workspace");
        let config_json = format!(
            r#"{{
  "providers": {{
    "openai": {{ "api_key": "sk-test-key", "api_base": "{new_uri}" }}
  }},
  "agents": {{
    "defaults": {{
      "workspace": "{workspace}"
    }}
  }}
}}"#,
            new_uri = new_uri,
            workspace = workspace.display()
        );
        std::fs::write(base.join("config.json"), config_json).expect("rewrite config");
        world._wiremock_server_uri = Some(new_uri);
        std::mem::forget(server);
    });
    std::mem::forget(rt);
}

#[given("no config file exists")]
fn given_no_config_file(world: &mut QuectoWorld) {
    // Delete config.json if the temp dir already has one (from prior Given steps).
    let config_path = base_path(world).join("config.json");
    if config_path.exists() {
        std::fs::remove_file(&config_path).expect("remove config.json");
    }
}

#[given("a config file with no API keys")]
fn given_config_no_api_keys(world: &mut QuectoWorld) {
    ensure_temp_dir(world);
    let base = base_path(world);
    let config_json = r#"{
  "providers": {
    "openai": { "api_key": "" },
    "anthropic": { "api_key": "" }
  }
}"#;
    std::fs::write(base.join("config.json"), config_json).expect("write config");
}

/// Generic step: "When I run quecto agent ..." with arbitrary flags.
/// Parses the full argument string after "quecto " using shell-like splitting.
#[when(expr = "I run quecto agent -m {string}")]
fn when_run_agent_with_message(world: &mut QuectoWorld, message: String) {
    let args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "-m".to_string(),
        message,
    ];
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

#[when("I run quecto agent with no flags")]
fn when_run_agent_no_flags(world: &mut QuectoWorld) {
    let args = vec!["quecto".to_string(), "agent".to_string()];
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

#[when(expr = "I run quecto agent --system {string} -m {string}")]
fn when_run_agent_with_system_and_message(
    world: &mut QuectoWorld,
    system: String,
    message: String,
) {
    let args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "--system".to_string(),
        system,
        "-m".to_string(),
        message,
    ];
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

#[when(expr = "I run quecto agent --model {word} -m {string}")]
fn when_run_agent_with_model_and_message(world: &mut QuectoWorld, model: String, message: String) {
    let args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "--model".to_string(),
        model,
        "-m".to_string(),
        message,
    ];
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

#[when("I set QUECTO_BASE_DIR to the temp directory")]
fn when_set_quecto_base_dir_env(world: &mut QuectoWorld) {
    let base = base_path(world);
    // Set the real env var — safe because BDD runs single-threaded (max_concurrent_scenarios(1)).
    // The production CliContext::base_dir() reads QUECTO_BASE_DIR from the environment.
    // SAFETY: No concurrent threads are reading env vars in the BDD test runner.
    unsafe {
        std::env::set_var("QUECTO_BASE_DIR", base.to_string_lossy().as_ref());
    }
    // Track for cleanup in ensure_temp_dir (next scenario init).
    world.env_base_dir_set = true;
    // Also clear cli_context.base_dir so the code must use the env var.
    world.cli_context.base_dir = None;
}

#[then(expr = "stdout should contain {string}")]
fn then_stdout_contains(world: &mut QuectoWorld, expected: String) {
    assert!(
        world.stdout.contains(&expected),
        "expected stdout to contain '{}', got: {}",
        expected,
        world.stdout
    );
}

#[then(expr = "stderr should contain {string}")]
fn then_stderr_contains_e2e(world: &mut QuectoWorld, expected: String) {
    assert!(
        world.stderr.contains(&expected),
        "expected stderr to contain '{}', got: {}",
        expected,
        world.stderr
    );
}

// ===========================================================================
// E2E Tool Use + E2E Session Steps
// ===========================================================================

/// Helper: build the OpenAI-format JSON for a tool call response.
fn openai_tool_call_json(tool_name: &str, args_json: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "chatcmpl-tool",
        "object": "chat.completion",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": format!("call_{}", tool_name),
                    "type": "function",
                    "function": {
                        "name": tool_name,
                        "arguments": args_json
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": { "prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15 }
    })
}

/// Helper: build the OpenAI-format JSON for a text response.
fn openai_text_json(content: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "chatcmpl-text",
        "object": "chat.completion",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": content
            },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15 }
    })
}

/// Helper: rewrite config to point at a new wiremock URI (shared pattern).
fn rewrite_config_to_uri(world: &mut QuectoWorld, new_uri: &str) {
    let base = base_path(world);
    let workspace = base.join("workspace");
    let config_json = format!(
        r#"{{
  "providers": {{
    "openai": {{ "api_key": "sk-test-key", "api_base": "{new_uri}" }}
  }},
  "agents": {{
    "defaults": {{
      "workspace": "{workspace}"
    }}
  }}
}}"#,
        new_uri = new_uri,
        workspace = workspace.display()
    );
    std::fs::write(base.join("config.json"), config_json).expect("rewrite config");
    world._wiremock_server_uri = Some(new_uri.to_string());
}

/// Helper: mount a two-response wiremock sequence — first a tool call, then a text response.
/// Uses priority: tool call at priority 2 with up_to_n_times(1), text at priority 1 (default).
fn mount_tool_then_text_sequence(
    world: &mut QuectoWorld,
    tool_name: &str,
    args_json: &str,
    final_text: &str,
) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();

        // First call: return tool call (higher priority, consumed once)
        let tool_body = openai_tool_call_json(tool_name, args_json);
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(tool_body))
            .up_to_n_times(1)
            .with_priority(1) // higher priority (lower number = higher priority in wiremock)
            .mount(&server)
            .await;

        // Second call onward: return text response (lower priority)
        let text_body = openai_text_json(final_text);
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(text_body))
            .with_priority(2)
            .mount(&server)
            .await;

        rewrite_config_to_uri(world, &new_uri);
        std::mem::forget(server);
    });
    std::mem::forget(rt);
}

// --- Given: e2e workspace file creation ---

#[given(expr = "a file {string} in the e2e workspace with content {string}")]
fn given_file_in_e2e_workspace(world: &mut QuectoWorld, filename: String, content: String) {
    let base = base_path(world);
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let path = workspace.join(&filename);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent dirs");
    }
    std::fs::write(&path, content).expect("write file");
}

// --- Given: wiremock tool-call mocks ---

#[given(expr = "the mock LLM first returns a tool call for {string} with args:")]
fn given_mock_llm_tool_call(world: &mut QuectoWorld, step: &gherkin::Step, tool_name: String) {
    // Parse args from the Gherkin table into a JSON object.
    let table = step.table.as_ref().expect("step should have a table");
    let mut map = serde_json::Map::new();
    for row in &table.rows {
        if row.len() >= 2 {
            map.insert(row[0].clone(), serde_json::Value::String(row[1].clone()));
        }
    }
    let args_json = serde_json::to_string(&map).unwrap();
    // Store for later pairing with the "then returns text" step.
    world.pending_tool_call = Some((tool_name, args_json));
}

#[given(expr = "the mock LLM then returns a text response {string}")]
fn given_mock_llm_then_text_response(world: &mut QuectoWorld, content: String) {
    if let Some((tool_name, args_json)) = world.pending_tool_call.take() {
        mount_tool_then_text_sequence(world, &tool_name, &args_json, &content);
    } else {
        // No pending tool call — just mount a plain text response (same as existing step).
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let server = wiremock::MockServer::start().await;
            let new_uri = server.uri();
            let body = openai_text_json(&content);
            wiremock::Mock::given(wiremock::matchers::method("POST"))
                .and(wiremock::matchers::path("/chat/completions"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(body))
                .mount(&server)
                .await;
            rewrite_config_to_uri(world, &new_uri);
            std::mem::forget(server);
        });
        std::mem::forget(rt);
    }
}

// Multi-turn tool call sequence from a table:
//   | call | read_file  | {"path":"source.txt"} |
//   | call | write_file | {"path":"copy.txt","content":"data"} |
//   | text | Done       |                       |
#[given("the mock LLM returns a tool call sequence:")]
fn given_mock_llm_tool_call_sequence(world: &mut QuectoWorld, step: &gherkin::Step) {
    let table = step.table.as_ref().expect("step should have a table");

    // Collect responses in order.
    let mut responses: Vec<serde_json::Value> = Vec::new();
    for row in &table.rows {
        let kind = &row[0];
        match kind.as_str() {
            "call" => {
                let tool_name = &row[1];
                let args_json = &row[2];
                responses.push(openai_tool_call_json(tool_name, args_json));
            }
            "text" => {
                let content = &row[1];
                responses.push(openai_text_json(content));
            }
            _ => panic!("Unknown sequence row kind: {kind}"),
        }
    }

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();

        // Mount each response with decreasing priority and up_to_n_times(1).
        // Priority 1 = highest (first consumed), then 2, etc. Last one has no limit.
        let last = responses.len() - 1;
        for (i, body) in responses.into_iter().enumerate() {
            let mock = wiremock::Mock::given(wiremock::matchers::method("POST"))
                .and(wiremock::matchers::path("/chat/completions"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(body))
                .with_priority(
                    u8::try_from(i + 1).expect("too many mock responses for u8 priority"),
                );
            if i < last {
                mock.up_to_n_times(1).mount(&server).await;
            } else {
                mock.mount(&server).await;
            }
        }

        rewrite_config_to_uri(world, &new_uri);
        std::mem::forget(server);
    });
    std::mem::forget(rt);
}

// --- Given: pre-existing session ---

#[given(expr = "a pre-existing session {string} with {int} messages")]
fn given_pre_existing_session(world: &mut QuectoWorld, key: String, count: usize) {
    let base = base_path(world);
    let sessions_dir = base.join("sessions");
    std::fs::create_dir_all(&sessions_dir).expect("create sessions dir");

    // Build a session file with the requested number of user/assistant message pairs.
    let mut messages = Vec::new();
    for i in 0..count {
        let role = if i % 2 == 0 { "user" } else { "assistant" };
        let content = format!("message {}", i + 1);
        messages.push(serde_json::json!({
            "role": role,
            "content": content
        }));
    }
    let session_file = serde_json::json!({
        "key": key,
        "messages": messages
    });
    // The filename uses : -> _ replacement and .json suffix.
    let filename = key.replace(':', "_") + ".json";
    std::fs::write(
        sessions_dir.join(&filename),
        serde_json::to_string_pretty(&session_file).unwrap(),
    )
    .expect("write session file");
}

// --- When: run agent with session flags ---

#[when(expr = "I run quecto agent -s {word} -m {string}")]
fn when_run_agent_named_session(world: &mut QuectoWorld, session: String, message: String) {
    let args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "-s".to_string(),
        session,
        "-m".to_string(),
        message,
    ];
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

#[when(expr = "I run quecto agent -s {word} --system {string} -m {string}")]
fn when_run_agent_session_system(
    world: &mut QuectoWorld,
    session: String,
    system: String,
    message: String,
) {
    let args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "-s".to_string(),
        session,
        "--system".to_string(),
        system,
        "-m".to_string(),
        message,
    ];
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

// --- Then: e2e workspace file assertions ---

#[then(expr = "the file {string} should exist in the e2e workspace")]
fn then_file_exists_in_e2e_workspace(world: &mut QuectoWorld, filename: String) {
    let base = base_path(world);
    let path = base.join("workspace").join(&filename);
    assert!(
        path.exists(),
        "expected file '{}' to exist at {}",
        filename,
        path.display()
    );
}

#[then(expr = "the file {string} in the e2e workspace should contain {string}")]
fn then_file_in_e2e_workspace_contains(
    world: &mut QuectoWorld,
    filename: String,
    expected: String,
) {
    let base = base_path(world);
    let path = base.join("workspace").join(&filename);
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read '{}' at {}: {}", filename, path.display(), e));
    assert!(
        content.contains(&expected),
        "expected '{}' to contain '{}', got: {}",
        filename,
        expected,
        content
    );
}

// --- Then: session file assertions ---

#[then(expr = "a session file should exist for key {string}")]
fn then_session_file_exists(world: &mut QuectoWorld, key: String) {
    let base = base_path(world);
    let filename = key.replace(':', "_") + ".json";
    let path = base.join("sessions").join(&filename);
    assert!(
        path.exists(),
        "expected session file for key '{}' at {}",
        key,
        path.display()
    );
}

#[then(expr = "the session {string} should contain {int} messages")]
fn then_session_has_n_messages(world: &mut QuectoWorld, key: String, expected: usize) {
    let session = load_session_from_disk(world, &key);
    assert_eq!(
        session.messages.len(),
        expected,
        "expected session '{}' to have {} messages, got {} (messages: {:?})",
        key,
        expected,
        session.messages.len(),
        session
            .messages
            .iter()
            .map(|m| format!("{}:{}", m.role, &m.content[..m.content.len().min(40)]))
            .collect::<Vec<_>>()
    );
}

#[then(expr = "the session {string} should contain at least {int} messages")]
fn then_session_has_at_least_n_messages(world: &mut QuectoWorld, key: String, expected: usize) {
    let session = load_session_from_disk(world, &key);
    assert!(
        session.messages.len() >= expected,
        "expected session '{}' to have at least {} messages, got {}",
        key,
        expected,
        session.messages.len()
    );
}

#[then(expr = "the session {string} should not contain text {string}")]
fn then_session_not_contain_text(world: &mut QuectoWorld, key: String, text: String) {
    let session = load_session_from_disk(world, &key);
    let all_content: String = session
        .messages
        .iter()
        .map(|m| m.content.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        !all_content.contains(&text),
        "expected session '{}' to NOT contain '{}', but found it in: {}",
        key,
        text,
        all_content
    );
}

#[then("no session files should exist")]
fn then_no_session_files(world: &mut QuectoWorld) {
    let base = base_path(world);
    let sessions_dir = base.join("sessions");
    if !sessions_dir.exists() {
        return; // No sessions dir = no session files, as expected.
    }
    let entries: Vec<_> = std::fs::read_dir(&sessions_dir)
        .expect("read sessions dir")
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "json")
                .unwrap_or(false)
        })
        .collect();
    assert!(
        entries.is_empty(),
        "expected no session files, but found: {:?}",
        entries.iter().map(|e| e.file_name()).collect::<Vec<_>>()
    );
}

#[then(expr = "the session {string} should not include a system role message")]
fn then_session_no_system_messages(world: &mut QuectoWorld, key: String) {
    let session = load_session_from_disk(world, &key);
    let system_count = session
        .messages
        .iter()
        .filter(|m| m.role == "system")
        .count();
    assert_eq!(
        system_count, 0,
        "expected no system messages in session '{}', found {}",
        key, system_count
    );
}

/// Helper: load a session file from disk and parse it into a simple struct.
struct SessionOnDisk {
    messages: Vec<MessageOnDisk>,
}

struct MessageOnDisk {
    role: String,
    content: String,
}

fn load_session_from_disk(world: &QuectoWorld, key: &str) -> SessionOnDisk {
    let base = base_path(world);
    let filename = key.replace(':', "_") + ".json";
    let path = base.join("sessions").join(&filename);
    let data = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "failed to read session '{}' at {}: {}",
            key,
            path.display(),
            e
        )
    });
    let json: serde_json::Value = serde_json::from_str(&data)
        .unwrap_or_else(|e| panic!("failed to parse session '{}': {}", key, e));
    let messages = json["messages"]
        .as_array()
        .expect("session should have messages array")
        .iter()
        .map(|m| MessageOnDisk {
            role: m["role"].as_str().unwrap_or("").to_string(),
            content: m["content"].as_str().unwrap_or("").to_string(),
        })
        .collect();
    SessionOnDisk { messages }
}

// ===========================================================================
// E2E Agentic Loop Steps (parallel tool calls)
// ===========================================================================

/// Build an OpenAI-format JSON response with multiple parallel tool calls.
fn openai_parallel_tool_calls_json(calls: &[(String, String)]) -> serde_json::Value {
    let tool_calls: Vec<serde_json::Value> = calls
        .iter()
        .enumerate()
        .map(|(i, (name, args))| {
            serde_json::json!({
                "id": format!("call_{}_{}", name, i),
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": args
                }
            })
        })
        .collect();
    serde_json::json!({
        "id": "chatcmpl-parallel",
        "object": "chat.completion",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": tool_calls
            },
            "finish_reason": "tool_calls"
        }],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 5,
            "total_tokens": 15
        }
    })
}

#[given("the mock LLM returns parallel tool calls then text:")]
fn given_parallel_tool_calls(world: &mut QuectoWorld, step: &gherkin::Step) {
    let table = step.table.as_ref().expect("step should have a table");
    // Each row has pairs: tool_name, args_json, tool_name, args_json, ...
    let mut calls = Vec::new();
    for row in &table.rows {
        let mut i = 0;
        while i + 1 < row.len() {
            let name = row[i].clone();
            let args = row[i + 1].clone();
            if !name.is_empty() {
                calls.push((name, args));
            }
            i += 2;
        }
    }
    world.pending_parallel_calls = Some(calls);
}

#[given(expr = "the final text is {string}")]
fn given_final_text_for_parallel(world: &mut QuectoWorld, content: String) {
    let calls = world
        .pending_parallel_calls
        .take()
        .expect("no pending parallel calls");

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();

        // First response: parallel tool calls (higher priority)
        let body = openai_parallel_tool_calls_json(&calls);
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(body))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;

        // Second response: final text (lower priority)
        let text_body = openai_text_json(&content);
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(text_body))
            .with_priority(2)
            .mount(&server)
            .await;

        rewrite_config_to_uri(world, &new_uri);
        std::mem::forget(server);
    });
    std::mem::forget(rt);
}

// ===========================================================================
// E2E Safety and Limits Steps
// ===========================================================================

#[given("restrict_to_workspace is enabled in the config")]
fn given_restrict_to_workspace_enabled(world: &mut QuectoWorld) {
    let base = base_path(world);
    let config_str = std::fs::read_to_string(base.join("config.json")).expect("read config");
    let mut config: serde_json::Value = serde_json::from_str(&config_str).expect("parse config");
    config["agents"]["defaults"]["restrict_to_workspace"] = serde_json::Value::Bool(true);
    std::fs::write(
        base.join("config.json"),
        serde_json::to_string_pretty(&config).unwrap(),
    )
    .expect("rewrite config");
}

#[given(expr = "the config sets max_tool_iterations to {int}")]
fn given_config_max_tool_iterations(world: &mut QuectoWorld, max_iterations: u32) {
    let base = base_path(world);
    let config_str = std::fs::read_to_string(base.join("config.json")).expect("read config");
    let mut config: serde_json::Value = serde_json::from_str(&config_str).expect("parse config");
    config["agents"]["defaults"]["max_tool_iterations"] = serde_json::json!(max_iterations);
    std::fs::write(
        base.join("config.json"),
        serde_json::to_string_pretty(&config).unwrap(),
    )
    .expect("rewrite config");
}

#[given(expr = "the mock LLM always returns a tool call for {string} with args:")]
fn given_mock_llm_always_tool_call(
    world: &mut QuectoWorld,
    step: &gherkin::Step,
    tool_name: String,
) {
    let table = step.table.as_ref().expect("step should have a table");
    let mut map = serde_json::Map::new();
    for row in &table.rows {
        if row.len() >= 2 {
            map.insert(row[0].clone(), serde_json::Value::String(row[1].clone()));
        }
    }
    let args_json = serde_json::to_string(&map).unwrap();

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();

        let body = openai_tool_call_json(&tool_name, &args_json);
        // Mount with no limit — every request gets a tool call
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        rewrite_config_to_uri(world, &new_uri);
        std::mem::forget(server);
    });
    std::mem::forget(rt);
}

#[given(expr = "the mock LLM takes {int} seconds to respond")]
fn given_mock_llm_delayed_response(world: &mut QuectoWorld, delay_secs: u64) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();
        let body = openai_text_json("Delayed response");
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(body)
                    .set_delay(std::time::Duration::from_secs(delay_secs)),
            )
            .mount(&server)
            .await;

        rewrite_config_to_uri(world, &new_uri);
        std::mem::forget(server);
    });
    std::mem::forget(rt);
}

// --- When steps for --max-iterations and --max-time ---

#[when(expr = "I run quecto agent -s {word} --max-iterations {int} -m {string}")]
fn when_run_agent_max_iterations(
    world: &mut QuectoWorld,
    session: String,
    max_iterations: u32,
    message: String,
) {
    let args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "-s".to_string(),
        session,
        "--max-iterations".to_string(),
        max_iterations.to_string(),
        "-m".to_string(),
        message,
    ];
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

#[when(expr = "I run quecto agent -s {word} --max-time {int} -m {string}")]
fn when_run_agent_max_time(
    world: &mut QuectoWorld,
    session: String,
    max_time: u64,
    message: String,
) {
    let args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "-s".to_string(),
        session,
        "--max-time".to_string(),
        max_time.to_string(),
        "-m".to_string(),
        message,
    ];
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

// ===========================================================================
// E2E Provider Wiring Steps
// ===========================================================================

/// Helper: build the Anthropic Messages API response JSON for a text response.
fn anthropic_text_json(content: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "msg_test",
        "type": "message",
        "role": "assistant",
        "content": [{ "type": "text", "text": content }],
        "stop_reason": "end_turn",
        "usage": { "input_tokens": 10, "output_tokens": 5 }
    })
}

/// Helper: write a config file with the given provider entries.
fn write_provider_config(world: &mut QuectoWorld, openai_json: &str, anthropic_json: &str) {
    let base = base_path(world);
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let config_json = format!(
        r#"{{
  "providers": {{
    "openai": {openai_json},
    "anthropic": {anthropic_json}
  }},
  "agents": {{
    "defaults": {{
      "workspace": "{workspace}"
    }}
  }}
}}"#,
        openai_json = openai_json,
        anthropic_json = anthropic_json,
        workspace = workspace.display()
    );
    std::fs::write(base.join("config.json"), config_json).expect("write config");
}

#[given("a config file with an Anthropic provider pointing at a mock server")]
fn given_config_with_anthropic_mock(world: &mut QuectoWorld) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(wiremock::MockServer::start());
    let uri = server.uri();
    // Store in anthropic-specific field, NOT _wiremock_server_uri (OpenAI)
    world.wiremock_anthropic_uri = Some(uri.clone());

    ensure_temp_dir(world);
    let openai_json = r#"{ "api_key": "", "api_base": "" }"#;
    let anthropic_json = format!(r#"{{ "api_key": "sk-ant-test", "api_base": "{uri}" }}"#);
    write_provider_config(world, openai_json, &anthropic_json);

    std::mem::forget(server);
    std::mem::forget(rt);
}

#[given("a config file with both OpenAI and Anthropic providers pointing at mock servers")]
fn given_config_with_both_providers(world: &mut QuectoWorld) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (openai_uri, anthropic_uri) = rt.block_on(async {
        let s1 = wiremock::MockServer::start().await;
        let s2 = wiremock::MockServer::start().await;
        let u1 = s1.uri();
        let u2 = s2.uri();
        std::mem::forget(s1);
        std::mem::forget(s2);
        (u1, u2)
    });

    ensure_temp_dir(world);
    world._wiremock_server_uri = Some(openai_uri.clone());
    world.wiremock_anthropic_uri = Some(anthropic_uri.clone());

    let openai_json = format!(r#"{{ "api_key": "sk-test-key", "api_base": "{openai_uri}" }}"#);
    let anthropic_json =
        format!(r#"{{ "api_key": "sk-ant-test", "api_base": "{anthropic_uri}" }}"#);
    write_provider_config(world, &openai_json, &anthropic_json);

    std::mem::forget(rt);
}

/// Helper: rewrite config with a new OpenAI URI, preserving Anthropic if present.
fn rewrite_openai_in_config(world: &mut QuectoWorld, new_uri: &str) {
    let base = base_path(world);
    let workspace = base.join("workspace");
    let anthropic_uri = world.wiremock_anthropic_uri.as_deref().unwrap_or("");
    let anthropic_key = if anthropic_uri.is_empty() {
        ""
    } else {
        "sk-ant-test"
    };
    let config_json = format!(
        r#"{{
  "providers": {{
    "openai": {{ "api_key": "sk-test-key", "api_base": "{new_uri}" }},
    "anthropic": {{ "api_key": "{anthropic_key}", "api_base": "{anthropic_uri}" }}
  }},
  "agents": {{
    "defaults": {{
      "workspace": "{workspace}"
    }}
  }}
}}"#,
        new_uri = new_uri,
        anthropic_key = anthropic_key,
        anthropic_uri = anthropic_uri,
        workspace = workspace.display()
    );
    std::fs::write(base.join("config.json"), config_json).expect("rewrite config");
    world._wiremock_server_uri = Some(new_uri.to_string());
}

/// Helper: rewrite config with a new Anthropic URI, preserving OpenAI if present.
fn rewrite_anthropic_in_config(world: &mut QuectoWorld, new_uri: &str) {
    let base = base_path(world);
    let workspace = base.join("workspace");
    let openai_uri = world._wiremock_server_uri.as_deref().unwrap_or("");
    let openai_key = if openai_uri.is_empty() {
        ""
    } else {
        "sk-test-key"
    };
    let config_json = format!(
        r#"{{
  "providers": {{
    "openai": {{ "api_key": "{openai_key}", "api_base": "{openai_uri}" }},
    "anthropic": {{ "api_key": "sk-ant-test", "api_base": "{new_uri}" }}
  }},
  "agents": {{
    "defaults": {{
      "workspace": "{workspace}"
    }}
  }}
}}"#,
        openai_key = openai_key,
        openai_uri = openai_uri,
        new_uri = new_uri,
        workspace = workspace.display()
    );
    std::fs::write(base.join("config.json"), config_json).expect("rewrite config");
    world.wiremock_anthropic_uri = Some(new_uri.to_string());
}

#[given(expr = "the Anthropic mock returns an HTTP {int} error")]
fn given_anthropic_mock_error(world: &mut QuectoWorld, status: u16) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/v1/messages"))
            .respond_with(wiremock::ResponseTemplate::new(status).set_body_string("Error"))
            .mount(&server)
            .await;

        rewrite_anthropic_in_config(world, &new_uri);
        std::mem::forget(server);
    });
    std::mem::forget(rt);
}

#[given(expr = "the Anthropic mock returns a text response {string}")]
fn given_anthropic_mock_text_response(world: &mut QuectoWorld, content: String) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();
        let body = anthropic_text_json(&content);
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/v1/messages"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        rewrite_anthropic_in_config(world, &new_uri);
        std::mem::forget(server);
    });
    std::mem::forget(rt);
}

// --- Credential store integration steps ---

#[given(expr = "a config file with OpenAI api_key {string} pointing at a mock server")]
fn given_config_with_openai_custom_key(world: &mut QuectoWorld, api_key: String) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(wiremock::MockServer::start());
    let uri = server.uri();
    world._wiremock_server_uri = Some(uri.clone());

    ensure_temp_dir(world);
    let base = base_path(world);
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let config_json = format!(
        r#"{{
  "providers": {{
    "openai": {{ "api_key": "{api_key}", "api_base": "{uri}" }}
  }},
  "agents": {{
    "defaults": {{
      "workspace": "{workspace}"
    }}
  }}
}}"#,
        api_key = api_key,
        uri = uri,
        workspace = workspace.display()
    );
    std::fs::write(base.join("config.json"), config_json).expect("write config");

    std::mem::forget(server);
    std::mem::forget(rt);
}

#[given(expr = "the credential store has a valid token {string} for provider {string}")]
fn given_credential_store_valid_token(world: &mut QuectoWorld, token: String, provider: String) {
    let base = base_path(world);
    let store = CredentialStore::new(&base);
    store
        .store(Credential {
            provider,
            token,
            method: AuthMethod::Token,
            expires_at: None, // no expiry = always valid
        })
        .expect("store credential");
}

#[given(expr = "the credential store has an expired token {string} for provider {string}")]
fn given_credential_store_expired_token(world: &mut QuectoWorld, token: String, provider: String) {
    let base = base_path(world);
    let store = CredentialStore::new(&base);
    store
        .store(Credential {
            provider,
            token,
            method: AuthMethod::Token,
            expires_at: Some(0), // epoch = always expired
        })
        .expect("store credential");
}

#[given(expr = "the mock expects Authorization header {string} and returns {string}")]
fn given_mock_expects_auth_header(
    world: &mut QuectoWorld,
    expected_header: String,
    response_content: String,
) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();
        let body = openai_text_json(&response_content);
        // Mount mock that ONLY matches the expected Authorization header.
        // If the wrong token is sent, wiremock returns 404, causing failure.
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .and(wiremock::matchers::header(
                "Authorization",
                expected_header.as_str(),
            ))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        // Read existing config to preserve api_key, only replace api_base
        let base = base_path(world);
        let config_str =
            std::fs::read_to_string(base.join("config.json")).expect("read existing config");
        let mut config: serde_json::Value =
            serde_json::from_str(&config_str).expect("parse config");
        config["providers"]["openai"]["api_base"] = serde_json::Value::String(new_uri.clone());
        std::fs::write(
            base.join("config.json"),
            serde_json::to_string_pretty(&config).unwrap(),
        )
        .expect("rewrite config");
        world._wiremock_server_uri = Some(new_uri);
        std::mem::forget(server);
    });
    std::mem::forget(rt);
}

// --- Auth error steps ---

#[given(expr = "the OpenAI mock returns an HTTP {int} error")]
fn given_openai_mock_http_error(world: &mut QuectoWorld, status: u16) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(status).set_body_string("Error"))
            .mount(&server)
            .await;

        rewrite_openai_in_config(world, &new_uri);
        std::mem::forget(server);
    });
    std::mem::forget(rt);
}

// ===========================================================================
// E2E Subprocess Protocol Steps
// ===========================================================================

/// Find the quecto binary path relative to the test executable.
/// The test binary is at `target/debug/deps/bdd-*`,
/// so `target/debug/quecto` is `../../quecto` relative to it.
fn quecto_binary_path() -> PathBuf {
    let test_exe = std::env::current_exe().expect("get current exe");
    let deps_dir = test_exe.parent().expect("deps dir");
    let debug_dir = deps_dir.parent().expect("debug dir");
    debug_dir.join("quecto")
}

/// Parse a shell-like argument string into individual args.
/// Handles double-quoted strings (e.g. `"Do the subtask"`).
///
/// Limitations: only handles double quotes; no single quotes,
/// backslash escapes, or nested quoting. Sufficient for the
/// hardcoded Gherkin step strings used in BDD scenarios.
fn shell_split(s: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for ch in s.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ' ' if !in_quotes => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        args.push(current);
    }
    args
}

/// Maximum wall-clock time (seconds) a subprocess may run before
/// the BDD test kills it. Prevents the suite from hanging forever.
const SUBPROCESS_TIMEOUT_SECS: u64 = 30;

/// Spawn quecto as a real subprocess, capturing output.
/// Sets QUECTO_BASE_DIR to the temp dir if cli_context has one,
/// otherwise inherits from the environment (for env-var tests).
/// Kills the child after [`SUBPROCESS_TIMEOUT_SECS`] if it has
/// not exited.
fn spawn_quecto_subprocess(world: &mut QuectoWorld, raw_args: &str) {
    let binary = quecto_binary_path();
    assert!(
        binary.exists(),
        "quecto binary not found at {}",
        binary.display()
    );
    let args = shell_split(raw_args);
    let mut cmd = std::process::Command::new(&binary);
    cmd.args(&args);
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    // If cli_context.base_dir is set, pass it explicitly.
    // Otherwise the env var is already set (e.g. by the
    // "I set QUECTO_BASE_DIR" step) and the child inherits it.
    if let Some(ref base) = world.cli_context.base_dir {
        cmd.env("QUECTO_BASE_DIR", base.to_string_lossy().as_ref());
    }

    let mut child = cmd.spawn().expect("spawn quecto subprocess");
    let start = std::time::Instant::now();
    let deadline = std::time::Duration::from_secs(SUBPROCESS_TIMEOUT_SECS);
    let poll_interval = std::time::Duration::from_millis(50);

    // Poll until the child exits or the deadline is reached.
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = child.wait_with_output().expect("collect subprocess output");
                world.subprocess_exit_code = Some(status.code().unwrap_or(-1));
                world.subprocess_stdout = Some(String::from_utf8_lossy(&out.stdout).into_owned());
                world.subprocess_stderr = Some(String::from_utf8_lossy(&out.stderr).into_owned());
                return;
            }
            Ok(None) => {
                if start.elapsed() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!(
                        "subprocess timed out after {}s \
                         (args: {})",
                        SUBPROCESS_TIMEOUT_SECS, raw_args
                    );
                }
                std::thread::sleep(poll_interval);
            }
            Err(e) => {
                panic!("failed to wait on subprocess: {e}");
            }
        }
    }
}

#[when(regex = r"^I spawn quecto as a subprocess with args: (.+)$")]
fn when_spawn_subprocess(world: &mut QuectoWorld, raw_args: String) {
    spawn_quecto_subprocess(world, &raw_args);
}

// "And the mock LLM returns a text response" after a When step
// is interpreted as a When step by cucumber-rs.
#[when(expr = "the mock LLM returns a text response {string}")]
fn when_mock_llm_text_response(world: &mut QuectoWorld, content: String) {
    given_mock_llm_text_response(world, content);
}

#[then(expr = "the subprocess exit code should be {int}")]
fn then_subprocess_exit_code(world: &mut QuectoWorld, expected: i32) {
    let actual = world
        .subprocess_exit_code
        .expect("no subprocess was spawned");
    assert_eq!(
        actual,
        expected,
        "expected subprocess exit code {}, got {}.\nstdout: {}\nstderr: {}",
        expected,
        actual,
        world.subprocess_stdout.as_deref().unwrap_or(""),
        world.subprocess_stderr.as_deref().unwrap_or("")
    );
}

#[then(expr = "the subprocess stdout should contain {string}")]
fn then_subprocess_stdout_contains(world: &mut QuectoWorld, expected: String) {
    let stdout = world
        .subprocess_stdout
        .as_ref()
        .expect("no subprocess was spawned");
    assert!(
        stdout.contains(&expected),
        "expected subprocess stdout to contain '{}', got: {}",
        expected,
        stdout
    );
}

#[then(expr = "the subprocess stderr should contain {string}")]
fn then_subprocess_stderr_contains(world: &mut QuectoWorld, expected: String) {
    let stderr = world
        .subprocess_stderr
        .as_ref()
        .expect("no subprocess was spawned");
    assert!(
        stderr.contains(&expected),
        "expected subprocess stderr to contain '{}', got: {}",
        expected,
        stderr
    );
}

// ===========================================================================
// E2E Real LLM Steps
// ===========================================================================

/// Set up a workspace configured to use a real OpenAI endpoint.
/// Reads OPENAI_API_KEY from the environment (required).
#[given("a real LLM workspace is configured")]
fn given_real_llm_workspace(world: &mut QuectoWorld) {
    ensure_temp_dir(world);
    let base = base_path(world);
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");

    let api_key =
        std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set for real LLM tests");

    let config_json = format!(
        r#"{{
  "providers": {{
    "openai": {{ "api_key": "{api_key}" }}
  }},
  "agents": {{
    "defaults": {{
      "workspace": "{workspace}"
    }}
  }}
}}"#,
        api_key = api_key,
        workspace = workspace.display()
    );
    std::fs::write(base.join("config.json"), config_json).expect("write real LLM config");
}

/// Run the agent against the real OpenAI endpoint with a cheap model and bounded iterations.
#[when(expr = "I run the real LLM agent with message {string}")]
fn when_run_real_llm_agent(world: &mut QuectoWorld, message: String) {
    let args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "--model".to_string(),
        "gpt-4o-mini".to_string(),
        "--max-iterations".to_string(),
        "5".to_string(),
        "-s".to_string(),
        "-".to_string(), // ephemeral session
        "-m".to_string(),
        message,
    ];
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

/// Assert that stdout is not empty (structural check for non-deterministic output).
#[then("stdout should not be empty")]
fn then_stdout_not_empty(world: &mut QuectoWorld) {
    assert!(
        !world.stdout.trim().is_empty(),
        "expected non-empty stdout, got empty.\nstderr: {}",
        world.stderr
    );
}

// ===========================================================================
// Runner
// ===========================================================================

fn main() {
    let real_llm_enabled = std::env::var("QUECTO_REAL_LLM").unwrap_or_default() == "1";

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
                // Include if feature or scenario is tagged @wip or @done
                feat.tags.iter().any(|t| t == "wip" || t == "done")
                    || sc.tags.iter().any(|t| t == "wip" || t == "done")
            }),
    );
}
