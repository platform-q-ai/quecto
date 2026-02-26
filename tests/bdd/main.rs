#![allow(private_interfaces)]

use cucumber::{World, gherkin, given, then, when};
use quecto::application::agent_loop::AgentLoopImpl;
use quecto::application::cron_executor;
use quecto::application::heartbeat::{self, HeartbeatResult, HeartbeatTask, HeartbeatTaskResult};
use quecto::application::subagent::{SubagentConfig, SubagentContext, validate_agent_id};
use quecto::application::voice as app_voice;
use quecto::domain::agent::{AgentInfo, AgentLoop, AgentResult};
use quecto::domain::coding_contract::{
    SeqScope, next_seq_for, validate_and_track_event_with_scope,
};
use quecto::domain::cron::{CronJob, CronJobResult, CronSchedule, CronStore};
use quecto::domain::error::DomainError;
use quecto::domain::message::{LlmResponse, Message, Role, ToolCall};
use quecto::domain::provider::LlmProvider;
use quecto::domain::session::{ContextSpillStore, Session, SessionStore};
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

use quecto::application::coding_coordinator::{
    CodingCoordinator, CoordinatorPolicy, FailureInfo, RepoValidator, SkillResolver, SuccessInfo,
};

// ===========================================================================
// BDD mock implementations for CodingCoordinator ports
// ===========================================================================

#[derive(Debug, Clone, Default)]
struct BddRepoValidator {
    valid_repos: Vec<String>,
    valid_refs: Vec<(String, String)>,
}

impl RepoValidator for BddRepoValidator {
    fn repo_exists(&self, repo: &str) -> bool {
        self.valid_repos.iter().any(|r| r == repo)
    }
    fn ref_exists(&self, repo: &str, base_ref: &str) -> bool {
        self.valid_refs
            .iter()
            .any(|(r, b)| r == repo && b == base_ref)
    }
}

#[derive(Debug, Clone, Default)]
struct BddSkillResolver {
    available: Vec<String>,
}

impl SkillResolver for BddSkillResolver {
    fn skill_exists(&self, name: &str) -> bool {
        self.available.iter().any(|s| s == name)
    }
}

use quecto::domain::coding_ports::{
    CreatePrParams, GitHubPort, GitPrMutationResult, GitPrResult, GitPrStatusSummary, GitPushResult,
};

/// BDD mock for GitHub API operations (publish scenarios).
#[derive(Debug, Clone)]
struct BddGitHubPort {
    push_ok: bool,
    push_error: Option<String>,
    branch_protected: bool,
    branch_protected_err: Option<String>,
    create_pr_ok: bool,
    create_pr_number: Option<u64>,
    create_pr_url: Option<String>,
    create_pr_error: Option<String>,
    mutation_ok: bool,
    pr_status_ok: bool,
}

impl Default for BddGitHubPort {
    fn default() -> Self {
        Self {
            push_ok: true,
            push_error: None,
            branch_protected: false,
            branch_protected_err: None,
            create_pr_ok: true,
            create_pr_number: Some(123),
            create_pr_url: Some("https://github.com/org/repo/pull/123".to_string()),
            create_pr_error: None,
            mutation_ok: true,
            pr_status_ok: true,
        }
    }
}

impl GitHubPort for BddGitHubPort {
    fn push_branch(&self, _repo: &str, _branch: &str, _force: bool) -> GitPushResult {
        GitPushResult {
            ok: self.push_ok,
            error: self.push_error.clone(),
        }
    }

    fn is_branch_protected(&self, _repo: &str, _branch: &str) -> Result<bool, String> {
        if let Some(err) = &self.branch_protected_err {
            return Err(err.clone());
        }
        Ok(self.branch_protected)
    }

    fn create_pr(&self, _params: &CreatePrParams) -> GitPrResult {
        GitPrResult {
            ok: self.create_pr_ok,
            pr_number: self.create_pr_number,
            url: self.create_pr_url.clone(),
            error: self.create_pr_error.clone(),
        }
    }

    fn update_pr(&self, _repo: &str, _pr: u64, _body: Option<&str>) -> GitPrMutationResult {
        GitPrMutationResult {
            ok: self.mutation_ok,
            error: None,
        }
    }

    fn request_review(&self, _repo: &str, _pr: u64, _reviewers: &[String]) -> GitPrMutationResult {
        GitPrMutationResult {
            ok: self.mutation_ok,
            error: None,
        }
    }

    fn add_labels(&self, _repo: &str, _pr: u64, _labels: &[String]) -> GitPrMutationResult {
        GitPrMutationResult {
            ok: self.mutation_ok,
            error: None,
        }
    }

    fn get_pr_status(&self, _repo: &str, _pr: u64) -> GitPrStatusSummary {
        GitPrStatusSummary {
            ok: self.pr_status_ok,
            state: Some("open".to_string()),
            review_state: Some("approved".to_string()),
            checks_passed: Some(true),
            error: None,
        }
    }
}

use quecto::infrastructure::persistence::cron_store::FileCronStore;
use quecto::infrastructure::persistence::memory_store::{self, MemoryStore};
use quecto::infrastructure::persistence::session_store::FileSessionStore;
use quecto::infrastructure::persistence::skill_loader::FileSkillLoader;
use quecto::infrastructure::providers;
use quecto::infrastructure::providers::error::ErrorClass;
use quecto::infrastructure::providers::fallback::FallbackProvider;
use quecto::infrastructure::security::sandbox::Sandbox;
use quecto::infrastructure::tools::exec::{ExecIsolationMode, ExecTool};
use quecto::infrastructure::tools::message::MessageTool;
use quecto::infrastructure::tools::registry::ToolRegistryImpl;
use quecto::infrastructure::tools::spawn::SpawnTool;
use quecto::infrastructure::tools::wasm::host::SentMessage;
use quecto::infrastructure::tools::wasm::loader;
use quecto::infrastructure::tools::wasm::runtime::WasmToolRuntime;
use quecto::infrastructure::tools::wasm::wrapper::WasmToolWrapper;
use quecto::infrastructure::tools::web_search::WebSearchTool;
use quecto::infrastructure::voice::groq_whisper::{GroqWhisperClient, TranscriptionResult};
use quecto::interface::cli::{self, CliContext};
use quecto::interface::gateway::handle_bot_command;
use std::collections::HashMap;
use std::fs;
use std::future::Future;
use std::hash::{Hash, Hasher};
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

// Wrapper for Arc<dyn ContextSpillStore> that implements Debug (opaque).
#[derive(Clone)]
struct DebugSpillStore(Arc<dyn ContextSpillStore>);

impl std::fmt::Debug for DebugSpillStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<SpillStore>")
    }
}

impl std::ops::Deref for DebugSpillStore {
    type Target = Arc<dyn ContextSpillStore>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

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
    /// Leaked wiremock server for skills install mock API scenarios
    pub github_mock_server: Option<&'static wiremock::MockServer>,
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
    /// nsjail BDD: whether nsjail availability was declared by scenario
    pub nsjail_available: Option<bool>,
    /// nsjail BDD: selected binary path used to construct ExecTool in nsjail mode
    pub nsjail_binary: Option<String>,
    /// nsjail BDD: startup warning from exec tool fallback
    pub nsjail_startup_warning: Option<String>,
    /// nsjail BDD: measured execution elapsed milliseconds
    pub nsjail_elapsed_ms: Option<u128>,
    /// nsjail BDD: marker file written by fake nsjail script on invocation
    pub nsjail_invocation_marker: Option<PathBuf>,
    /// nsjail BDD: requested exec mode after registry construction
    pub nsjail_registry_mode: Option<ExecIsolationMode>,
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
    /// If true, the next "run quecto agent" step should run as subprocess
    /// with QUECTO_BASE_DIR set in child env to exercise env-based resolution
    /// without mutating process-global env in this test runner.
    pub run_agent_via_subprocess_with_env_base_dir: bool,
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
    /// Context pruning: in-memory spill store
    pub context_spill_store: Option<DebugSpillStore>,
    /// Context pruning: messages under test
    pub context_messages: Option<Vec<Message>>,
    /// Context pruning: current turn counter
    pub context_current_turn: Option<u32>,
    /// Context pruning: saved original tool content
    pub context_original_tool_content: Option<String>,
    /// Context pruning: last recall tool result
    pub context_recall_result: Option<ToolResult>,
    /// Context pruning: repeated recall count
    pub context_recall_count: Option<u32>,
    /// Context pruning: max context token budget for tests
    pub context_max_tokens: Option<usize>,
    /// Context pruning: temp dir for session persistence round-trip tests
    pub context_temp_dir: Option<TempDir>,
    /// Gateway subprocess child process handle (for long-running gateway tests)
    pub gateway_child: Option<std::process::Child>,
    /// Health server port assigned for gateway health e2e tests
    pub gateway_health_port: Option<u16>,
    /// Leaked wiremock server for gateway voice/spawn e2e tests (unified Telegram + Groq mock)
    pub gateway_mock_server: Option<&'static wiremock::MockServer>,
    /// Groq Whisper mock server (separate from Telegram mock) for voice e2e tests
    pub groq_mock_server: Option<&'static wiremock::MockServer>,
    // --- WASM tool runtime BDD fields ---
    /// WASM tool runtime instance
    pub wasm_runtime: Option<Arc<WasmToolRuntime>>,
    /// WASM tool wrapper for execution tests
    pub wasm_wrapper: Option<Arc<WasmToolWrapper>>,
    /// WASM execution results (for multi-execution tests)
    pub wasm_execution_results:
        Option<Vec<Result<quecto::domain::tool::ToolResult, quecto::domain::error::DomainError>>>,
    /// WASM single execution result
    pub wasm_single_result:
        Option<Result<quecto::domain::tool::ToolResult, quecto::domain::error::DomainError>>,
    /// WASM host result (from direct host function calls)
    pub wasm_host_result: Option<Result<String, String>>,
    /// WASM workspace path for host tests
    pub wasm_workspace: Option<PathBuf>,
    /// WASM temp dir (kept alive)
    pub _wasm_temp_dir: Option<TempDir>,
    /// WASM HTTP allowlist
    pub wasm_http_allowlist: Option<Vec<String>>,
    /// WASM HTTP stubs (host -> response)
    pub wasm_http_stubs: HashMap<String, String>,
    /// WASM sent messages (from send-message host calls)
    pub wasm_sent_messages: Option<Vec<SentMessage>>,
    /// WASM cron operations performed
    pub wasm_cron_ops: Option<Vec<quecto::infrastructure::tools::wasm::host::StoreOp>>,
    /// WASM spill data (pre-loaded for recall)
    pub wasm_spill_data: HashMap<String, String>,
    /// WASM log count (after rate-limited logging)
    pub wasm_log_count: Option<usize>,
    /// WASM tool registry for wrapper integration tests
    pub wasm_tool_registry: Option<ToolRegistryImpl>,
    /// WASM tools directory for loader tests
    pub wasm_tools_dir: Option<PathBuf>,
    /// WASM load result from directory scan
    pub wasm_load_result: Option<Result<loader::LoadResult, String>>,
    /// WASM registration error (for missing exports scenario)
    pub wasm_registration_error: Option<String>,
    // --- WASM tool port BDD fields ---
    /// WASM tool port: tool result from WASM execution
    pub wasm_tool_result: Option<quecto::domain::tool::ToolResult>,
    /// WASM tool port: native tool result for parity comparison
    pub wasm_native_result: Option<quecto::domain::tool::ToolResult>,
    /// WASM tool port: native tool registry for parity tests
    pub wasm_native_registry: Option<ToolRegistryImpl>,
    /// WASM tool port: WASM tool registry for port tests
    pub wasm_port_registry: Option<ToolRegistryImpl>,
    /// WASM tool port: shared workspace path for parity tests
    pub wasm_parity_workspace: Option<PathBuf>,
    /// WASM tool port: temp dir for parity tests
    pub _wasm_parity_temp_dir: Option<TempDir>,
    // --- Coding job lifecycle BDD fields ---
    /// Coding coordinator (application layer) for lifecycle scenarios
    pub coding_coordinator: Option<CodingCoordinator<BddRepoValidator, BddSkillResolver>>,
    /// Last job_id returned by run() — used to reference the current job
    pub coding_current_job_id: Option<String>,
    /// Current coding job under test
    pub coding_job: Option<quecto::domain::coding_job::CodingJob>,
    /// Multiple coding jobs (for list command scenarios)
    pub coding_jobs: Vec<quecto::domain::coding_job::CodingJob>,
    /// Emitted coding events during the current scenario
    pub coding_events: Vec<quecto::domain::coding_event::EventEnvelope>,
    /// Last seen event seq per `(source, run_id, job_id)`
    pub coding_event_seq_by_source_job: HashMap<SeqScope, u64>,
    /// Last command error from a coding command
    pub coding_command_error: Option<quecto::domain::coding_command::CommandError>,
    /// Last run response
    pub coding_run_response: Option<quecto::domain::coding_command::RunResponse>,
    /// Last status response
    pub coding_status_response: Option<quecto::domain::coding_command::StatusResponse>,
    /// Last cancel response
    pub coding_cancel_response: Option<quecto::domain::coding_command::CancelResponse>,
    /// Last cleanup response
    pub coding_cleanup_response: Option<quecto::domain::coding_command::CleanupResponse>,
    /// Last list response
    pub coding_list_response: Option<quecto::domain::coding_command::ListResponse>,
    /// Coding job temp directory (for cleanup assertions)
    pub coding_job_dir: Option<PathBuf>,
    /// Coding job temp dir handle (kept alive)
    pub _coding_temp_dir: Option<TempDir>,
    /// Skill policy: allowlist
    pub coding_skill_allowlist: Vec<String>,
    /// Skill policy: denylist
    pub coding_skill_denylist: Vec<String>,
    /// Skill policy: whether coordinator injects skills
    pub coding_skill_injection_enabled: bool,
    /// Skill policy: default skills merged into each job
    pub coding_skill_defaults: Vec<String>,
    /// Profile to skills mapping
    pub coding_profile_skills: HashMap<String, Vec<String>>,
    /// Profile-specific allowlist overrides
    pub coding_profile_allowlist: HashMap<String, Vec<String>>,
    /// Profile-specific denylist overrides
    pub coding_profile_denylist: HashMap<String, Vec<String>>,
    /// Skills marked as missing from disk for scenario simulation
    pub coding_missing_skill_files: Vec<String>,
    /// Effective applied skills for the current job
    pub coding_effective_skills: Vec<String>,
    /// Last requested skills in run invocation
    pub coding_requested_skills: Vec<String>,
    /// Snapshot reference produced by skills injection
    pub coding_skills_snapshot_ref: Option<String>,
    /// skills_applied artifact path produced at job start
    pub coding_skills_applied_artifact: Option<PathBuf>,
    /// Last profile selected for skill resolution
    pub coding_selected_profile: Option<String>,
    /// Whether worker context includes injected skill content
    pub coding_worker_context_has_skill_content: bool,
    /// Whether worker skill directory access was denied
    pub coding_worker_skill_access_denied: bool,
    /// Recorded skill suggestions from worker events
    pub coding_skill_suggestions: Vec<serde_json::Value>,
    /// Whether latest suggestion was policy denied
    pub coding_suggestion_policy_denied: bool,
    /// Last cleanup keep_artifacts flag used by scenario
    pub coding_keep_artifacts: bool,
    /// Last known state before cleanup removed the job.
    pub coding_last_cleaned_state: Option<quecto::domain::coding_job::JobState>,
    /// Whether a warning was logged in scenario simulation
    pub coding_warning_logged: bool,
    /// Whether a version mismatch error was logged in scenario simulation
    pub coding_version_error_logged: bool,
    /// Todo metadata: completion result by todo_id
    pub coding_todo_results: HashMap<String, String>,
    /// Todo metadata: blocked reason by todo_id
    pub coding_todo_blocked_reasons: HashMap<String, String>,
    /// Todo metadata: blocked needs by todo_id
    pub coding_todo_blocked_needs: HashMap<String, String>,
    /// Todo metadata: note by todo_id
    pub coding_todo_notes: HashMap<String, String>,
    /// Configured todo limit for current scenario
    pub coding_todo_max_items_per_job: Option<usize>,
    /// Whether latest todo transition was rejected
    pub coding_todo_transition_rejected: bool,
    /// Whether latest todo create was rejected
    pub coding_todo_create_rejected: bool,
    /// Recovery fixture event logs by job_id
    pub coding_recovery_logs: HashMap<String, Vec<serde_json::Value>>,
    /// Recovered states after coordinator startup replay
    pub coding_recovered_states: HashMap<String, String>,
    /// Recovered worker pid by job_id (if any)
    pub coding_recovered_worker_pid: HashMap<String, i64>,
    /// Simulated process liveness map for recovery checks
    pub coding_process_alive: HashMap<i64, bool>,
    /// Whether replay encountered truncated line and skipped it
    pub coding_truncated_line_skipped: bool,
    /// Whether replay encountered corrupted JSON and skipped it
    pub coding_corrupted_line_skipped: bool,
    /// Whether jobs index rewrite was performed
    pub coding_index_rewritten: bool,
    /// Whether worker recovery check was performed
    pub coding_worker_check_performed: bool,
    /// Whether coordinator startup failed due to lock
    pub coding_startup_failed_lock: bool,
    /// Number of appended recovery events
    pub coding_recovery_events_appended: usize,
    /// Whether state update happened after durable event flush
    pub coding_recovery_flush_then_state: bool,
    /// Observed operation order for recovery durability checks
    pub coding_recovery_operation_order: Vec<String>,
    /// Whether child spawn was marked failed during recovery
    pub coding_spawn_marked_failed: bool,
    /// Pending spawn policy to be applied after job creation (child agent scenarios)
    pub coding_pending_spawn_policy: Option<quecto::application::coding_spawn_manager::SpawnPolicy>,
    /// Publish coordinator (application layer) for GitHub publish scenarios
    pub coding_publish_coordinator:
        Option<quecto::application::coding_publish::PublishCoordinator<BddGitHubPort>>,
    /// Last publish result from a publish operation
    pub coding_publish_last_result: Option<quecto::application::coding_publish::PublishResult>,
    /// Startup error message captured when replay cannot begin
    pub coding_startup_error: Option<String>,
    // --- Coding job tool BDD fields ---
    /// CodingJobTool instance for tool-level BDD scenarios
    pub coding_job_tool: Option<Arc<quecto::infrastructure::tools::coding_job::CodingJobTool>>,
    /// Shared coordinator behind the CodingJobTool (for direct state manipulation)
    pub coding_job_tool_coordinator:
        Option<Arc<Mutex<CodingCoordinator<BddRepoValidator, BddSkillResolver>>>>,
    /// Last job_id from a CodingJobTool run action
    pub coding_job_tool_last_job_id: Option<String>,
    /// Last ToolResult from a CodingJobTool execution
    pub coding_job_tool_last_result: Option<quecto::domain::tool::ToolResult>,
    // --- Coding event persistence BDD fields ---
    pub coding_event_store:
        Option<quecto::infrastructure::persistence::coding_events::FileEventLogStore>,
    pub coding_event_store_dir: Option<PathBuf>,
    pub _coding_event_temp_dir: Option<TempDir>,
    pub coding_event_job_id: Option<String>,
    pub coding_event_n_jobs: Option<usize>,
    pub coding_event_last_appended: bool,
    pub coding_event_index_written: bool,
    pub coding_event_recovered_jobs: Option<Vec<(String, String)>>,
    pub coding_event_replayed_state: Option<String>,
    pub coding_event_replayed_progress: Option<u32>,
    pub coding_event_replayed_worker_pid: Option<u32>,
    pub coding_event_replayed_summary: Option<String>,
    pub coding_event_replay_had_corrupt: bool,
    pub coding_event_replay_had_truncated: bool,
    pub coding_event_truncated_present: bool,
    pub coding_event_corrupted_present: bool,
    pub coding_event_oversized_attempted: bool,
    pub coding_event_discovered_jobs: Option<Vec<String>>,
    pub coding_event_discovery_dirs: Option<Vec<String>>,
    pub coding_event_lock_acquired: Option<bool>,
    pub coding_event_read_lines: Option<Vec<quecto::domain::coding_ports::EventLogLine>>,
    // --- Coding repo mirror BDD fields ---
    pub coding_mirror_store:
        Option<quecto::infrastructure::coding::repo_mirror::FileRepoMirrorStore>,
    pub coding_mirror_cache_dir: Option<PathBuf>,
    pub _coding_mirror_temp_dir: Option<TempDir>,
    pub _coding_mirror_origin_dir: Option<TempDir>,
    pub coding_mirror_origin_path: Option<PathBuf>,
    pub coding_mirror_repo: Option<String>,
    pub coding_mirror_job_id: Option<String>,
    pub coding_mirror_last_result: Option<quecto::domain::coding_ports::RepoOpResult>,
    pub coding_mirror_created: bool,
    pub coding_mirror_fetched: bool,
    pub coding_mirror_cloned: bool,
    pub coding_mirror_clone_waited: bool,
    pub coding_mirror_fetch_waited: bool,
    pub coding_mirror_stale_lock_released: bool,
    // --- Coding worker runtime BDD fields ---
    pub coding_worker_runtime:
        Option<quecto::infrastructure::coding::worker_runtime::MockWorkerRuntime>,
    pub coding_worker_launch_config: Option<quecto::domain::coding_ports::WorkerLaunchConfig>,
    pub coding_worker_pid: Option<u32>,
    pub coding_worker_pids: Option<Vec<u32>>,
    pub coding_worker_job_state: Option<String>,
    pub coding_worker_exit_status: Option<i32>,
    pub coding_worker_last_event: Option<quecto::domain::coding_ports::WorkerEvent>,
    pub coding_worker_command_sent: bool,
    pub coding_worker_malformed_detected: bool,
    pub coding_worker_timeout_fired: bool,
    pub coding_worker_user_canceled: bool,
    pub coding_worker_last_exec_cmd: Option<String>,
    pub coding_worker_max_parallel: Option<usize>,
    pub coding_worker_queued_count: Option<usize>,
    pub coding_worker_third_job_queued: bool,
    // --- Nonblocking coordinator BDD fields ---
    pub nb_coord_bus: Option<quecto::infrastructure::coding::coordinator_bus::CoordinatorBus>,
    pub nb_coord_sender: Option<
        tokio::sync::mpsc::Sender<
            quecto::infrastructure::coding::coordinator_bus::CoordinatorCommand,
        >,
    >,
    pub nb_coord_handle: Option<quecto::infrastructure::coding::coordinator_bus::CoordinatorHandle>,
    pub nb_coord_last_cmd:
        Option<quecto::infrastructure::coding::coordinator_bus::CoordinatorCommand>,
    pub nb_coord_last_response:
        Option<quecto::infrastructure::coding::coordinator_bus::CoordinatorResponse>,
    pub nb_coord_reply_rx: Option<
        tokio::sync::oneshot::Receiver<
            quecto::infrastructure::coding::coordinator_bus::CoordinatorResponse,
        >,
    >,
    pub nb_coord_reply_rxs: Option<
        Vec<
            tokio::sync::oneshot::Receiver<
                quecto::infrastructure::coding::coordinator_bus::CoordinatorResponse,
            >,
        >,
    >,
    pub nb_coord_responses:
        Option<Vec<quecto::infrastructure::coding::coordinator_bus::CoordinatorResponse>>,
    pub nb_coord_dropped_reply_rx: Option<
        tokio::sync::oneshot::Receiver<
            quecto::infrastructure::coding::coordinator_bus::CoordinatorResponse,
        >,
    >,
    pub nb_coord_dispatch_mode:
        Option<quecto::infrastructure::coding::coordinator_bus::DispatchMode>,
    pub nb_coord_second_sent: bool,
    pub nb_coord_in_flight: bool,
    pub nb_coord_independent_done: bool,
    pub nb_coord_buffered_count: Option<usize>,
    // --- Worker coding tools BDD fields ---
    pub wct_job_dir: Option<PathBuf>,
    pub _wct_temp_dir: Option<TempDir>,
    pub wct_edit_result: Option<quecto::infrastructure::coding::worker_tools::EditResult>,
    pub wct_grep_result: Option<quecto::infrastructure::coding::worker_tools::GrepResult>,
    pub wct_find_result: Option<quecto::infrastructure::coding::worker_tools::FindResult>,
    pub wct_read_result: Option<quecto::infrastructure::coding::worker_tools::ReadResult>,
    pub wct_git_result: Option<quecto::infrastructure::coding::worker_tools::GitOpResult>,
    pub wct_preview_before: Option<String>,
    pub wct_blocked_command: Option<String>,
    pub wct_command_blocked: bool,
    pub wct_write_blocked: bool,
    pub wct_exec_ran: bool,
    pub wct_git_branch_name: Option<String>,
    // --- Coding artifact export BDD fields ---
    pub ae_job_dir: Option<PathBuf>,
    pub ae_artifacts_dir: Option<PathBuf>,
    pub ae_events: Vec<quecto::domain::coding_event::EventEnvelope>,
    pub _ae_temp_dir: Option<TempDir>,
    pub ae_export_result: Option<quecto::infrastructure::coding::artifact_export::ExportResult>,
    pub ae_status_artifacts: Option<Vec<String>>,
    pub ae_job_id: Option<String>,
    // --- Coding job operational BDD fields ---
    pub coding_operational_workspace: Option<PathBuf>,
    pub coding_operational_repo_ok: bool,
    pub coding_operational_ref_ok: bool,
    pub coding_operational_skill_ok: bool,
    pub coding_operational_registry:
        Option<quecto::infrastructure::tools::registry::ToolRegistryImpl>,
    pub coding_operational_definitions: Vec<quecto::domain::tool::ToolDefinition>,
    pub coding_operational_tool:
        Option<Arc<quecto::infrastructure::tools::coding_job::CodingJobTool>>,
    pub coding_operational_last_result: Option<quecto::domain::tool::ToolResult>,
    pub coding_operational_last_job_id: Option<String>,
    // --- Worker tool wrappers BDD fields ---
    pub wtw_job_dir: Option<PathBuf>,
    pub _wtw_temp_dir: Option<TempDir>,
    pub wtw_registry: Option<ToolRegistryImpl>,
    pub wtw_last_result: Option<Result<quecto::domain::tool::ToolResult, String>>,
    // --- Worker event emitter BDD fields ---
    pub wee_emitter:
        Option<quecto::infrastructure::coding::worker_event_emitter::WorkerEventEmitter<Vec<u8>>>,
    pub wee_last_emit_result: Option<Result<u64, String>>,
    // --- Worker entrypoint BDD fields ---
    pub cwe_parsed_args: Option<Result<quecto::interface::cli::worker::WorkerArgs, String>>,
    pub cwe_job_dir: Option<PathBuf>,
    pub _cwe_temp_dir: Option<TempDir>,
    pub cwe_validation_result: Option<Result<PathBuf, String>>,
    pub cwe_registry: Option<ToolRegistryImpl>,
    pub cwe_emitter:
        Option<quecto::infrastructure::coding::worker_event_emitter::WorkerEventEmitter<Vec<u8>>>,
    pub cwe_cli_stdout: Option<String>,
    pub cwe_cli_stderr: Option<String>,
    pub cwe_cli_exit_code: Option<i32>,
    // --- Real NsjailWorkerRuntime BDD fields ---
    pub nrt_runtime: Option<quecto::infrastructure::coding::nsjail_runtime::NsjailWorkerRuntime>,
    pub nrt_launch_config: Option<quecto::domain::coding_ports::WorkerLaunchConfig>,
    pub nrt_last_args: Option<Vec<String>>,
    pub nrt_last_worker_args: Option<Vec<String>>,
    pub nrt_run_id: Option<String>,
    pub nrt_job_id: Option<String>,
    pub nrt_pid: Option<u32>,
    pub nrt_resolved_binary: Option<String>,
    pub nrt_pids: Vec<u32>,
    pub nrt_last_error: Option<String>,
    // --- Coordinator-Worker Lifecycle BDD fields ---
    pub cwl_coordinator: Option<
        quecto::application::coding_coordinator::CodingCoordinator<
            BddRepoValidator,
            BddSkillResolver,
        >,
    >,
    pub cwl_worker_runtime:
        Option<quecto::infrastructure::coding::worker_runtime::MockWorkerRuntime>,
    pub cwl_job_ids: Vec<String>,
    pub cwl_worker_pids: Vec<u32>,
    pub cwl_clone_error: Option<String>,
    // --- Worker agent loop BDD fields ---
    pub wl_config: Option<quecto::application::worker_loop::WorkerLoopConfig>,
    pub wl_job_dir: Option<PathBuf>,
    pub wl_temp_dir: Option<TempDir>,
    pub wl_provider: Option<Arc<dyn LlmProvider>>,
    pub wl_result: Option<quecto::application::worker_loop::WorkerLoopResult>,
    pub wl_emitted_events: Vec<serde_json::Value>,
    pub wl_registry_names: Option<Vec<String>>,
    pub wl_emitter_run_id: Option<String>,
    pub wl_emitter_job_id: Option<String>,
    pub wl_system_prompt: Option<String>,
    pub wl_captured_messages: Arc<Mutex<Vec<Vec<Message>>>>,
    // --- Worker IPC integration BDD fields ---
    pub ipc_adapter: Option<
        Arc<quecto::infrastructure::coding::worker_event_emitter::WorkerEventSinkAdapter<Vec<u8>>>,
    >,
    pub ipc_last_emit_result: Option<Result<u64, String>>,
    pub ipc_emit_results: Vec<Result<u64, String>>,
    pub ipc_job_dir: Option<PathBuf>,
    pub ipc_temp_dir: Option<TempDir>,
    pub ipc_provider: Option<Arc<dyn LlmProvider>>,
    pub ipc_stdout: Option<String>,
    pub ipc_stderr: Option<String>,
    pub ipc_exit_code: Option<i32>,
}

fn push_coding_event(
    world: &mut QuectoWorld,
    source: quecto::domain::coding_event::EventSource,
    event_type: &str,
    payload: serde_json::Value,
) {
    let (run_id, job_id) = if let Some(j) = &world.coding_job {
        (j.run_id.clone(), j.job_id.clone())
    } else {
        ("run_abc123".to_string(), "job_abc123".to_string())
    };
    let scope = SeqScope::new(source, run_id.clone(), job_id.clone());
    let seq = next_seq_for(&scope, &world.coding_event_seq_by_source_job);
    let event = quecto::domain::coding_event::EventEnvelope {
        v: "1.0".to_string(),
        ts: "2026-01-01T00:00:00Z".to_string(),
        run_id,
        job_id,
        source,
        event_type: event_type.to_string(),
        seq,
        payload,
    };
    validate_and_track_event_with_scope(&event, scope, &mut world.coding_event_seq_by_source_job)
        .expect("coding event should satisfy contract");
    world.coding_events.push(event);
}

/// Ensure world has a temp dir and CliContext pointing to it.
fn ensure_temp_dir(world: &mut QuectoWorld) {
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

#[derive(Debug, Clone)]
struct ScenarioShardEntry {
    feature: String,
    scenario: String,
    weight: u64,
}

fn scenario_weight(tags: &[String], step_lines: &[String], feature: &str, scenario: &str) -> u64 {
    let mut w = 1_u64;
    if tags.iter().any(|t| t == "real-llm") {
        w += 4;
    }
    if tags.iter().any(|t| t == "real-llm-smoke") {
        w += 2;
    }
    if feature.to_ascii_lowercase().contains("gateway") {
        w += 2;
    }
    if scenario.to_ascii_lowercase().contains("gateway") {
        w += 2;
    }
    for line in step_lines {
        let l = line.to_ascii_lowercase();
        if l.contains("for at least 5 seconds") {
            w += 10;
        }
        if l.contains("takes 5 seconds") {
            w += 8;
        }
        if l.contains("wait for the health server to accept connections") {
            w += 6;
        }
        if l.contains("run the quecto gateway subprocess") {
            w += 4;
        }
    }
    w
}

fn discover_scenarios(features_dir: &str) -> Vec<ScenarioShardEntry> {
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(features_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "feature") {
                files.push(path);
            }
        }
    }
    files.sort();

    let mut out = Vec::new();
    for path in files {
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let mut feature_name = String::new();
        let mut pending_tags: Vec<String> = Vec::new();
        let mut current_scenario: Option<String> = None;
        let mut current_tags: Vec<String> = Vec::new();
        let mut current_steps: Vec<String> = Vec::new();

        let flush_current = |out: &mut Vec<ScenarioShardEntry>,
                             feature_name: &String,
                             current_scenario: &mut Option<String>,
                             current_tags: &mut Vec<String>,
                             current_steps: &mut Vec<String>| {
            if let Some(scenario_name) = current_scenario.take() {
                let weight =
                    scenario_weight(current_tags, current_steps, feature_name, &scenario_name);
                out.push(ScenarioShardEntry {
                    feature: feature_name.clone(),
                    scenario: scenario_name,
                    weight,
                });
                current_tags.clear();
                current_steps.clear();
            }
        };

        for raw_line in content.lines() {
            let line = raw_line.trim();
            if let Some(rest) = line.strip_prefix("Feature:") {
                feature_name = rest.trim().to_string();
                continue;
            }
            if line.starts_with('@') {
                pending_tags.extend(
                    line.split_whitespace()
                        .filter_map(|t| t.strip_prefix('@').map(str::to_string)),
                );
                continue;
            }
            if let Some(rest) = line
                .strip_prefix("Scenario:")
                .or_else(|| line.strip_prefix("Scenario Outline:"))
            {
                flush_current(
                    &mut out,
                    &feature_name,
                    &mut current_scenario,
                    &mut current_tags,
                    &mut current_steps,
                );
                current_scenario = Some(rest.trim().to_string());
                current_tags = std::mem::take(&mut pending_tags);
                continue;
            }
            if current_scenario.is_some() && !line.is_empty() {
                current_steps.push(line.to_string());
            }
        }

        flush_current(
            &mut out,
            &feature_name,
            &mut current_scenario,
            &mut current_tags,
            &mut current_steps,
        );
    }

    out
}

fn stable_hash(feature: &str, scenario: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    feature.hash(&mut hasher);
    scenario.hash(&mut hasher);
    hasher.finish()
}

fn build_shard_plan(features_dir: &str, shard_total: u64) -> HashMap<(String, String), u64> {
    let mut scenarios = discover_scenarios(features_dir);
    scenarios.sort_by(|a, b| {
        b.weight.cmp(&a.weight).then_with(|| {
            stable_hash(&a.feature, &a.scenario).cmp(&stable_hash(&b.feature, &b.scenario))
        })
    });

    let mut loads = vec![0_u64; shard_total as usize];
    let mut plan = HashMap::new();
    for s in scenarios {
        let key_hash = stable_hash(&s.feature, &s.scenario);
        let mut best_idx = 0_u64;
        let mut best_load = u64::MAX;
        let mut best_tie = u64::MAX;
        for idx in 0..shard_total {
            let load = loads[idx as usize];
            let tie = key_hash ^ idx;
            if load < best_load || (load == best_load && tie < best_tie) {
                best_load = load;
                best_tie = tie;
                best_idx = idx;
            }
        }
        loads[best_idx as usize] += s.weight;
        plan.insert((s.feature, s.scenario), best_idx);
    }
    plan
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
mod architecture_steps;
mod auth_steps;
mod coding_agent_responsiveness_steps;
mod coding_artifact_export_steps;
mod coding_child_agents_steps;
mod coding_coordinator_worker_lifecycle_steps;
mod coding_crash_recovery_steps;
mod coding_event_persistence_steps;
mod coding_github_publish_steps;
mod coding_job_lifecycle_steps;
mod coding_job_operational_steps;
mod coding_job_tool_steps;
mod coding_nonblocking_coordinator_steps;
mod coding_nsjail_runtime_process_steps;
mod coding_nsjail_runtime_steps;
mod coding_repo_mirror_steps;
mod coding_skills_steps;
mod coding_todos_steps;
mod coding_worker_coding_tools_steps;
mod coding_worker_entrypoint_steps;
mod coding_worker_event_emitter_steps;
mod coding_worker_ipc_steps;
mod coding_worker_loop_steps;
mod coding_worker_runtime_steps;
mod coding_worker_tool_wrappers_steps;
mod coding_worker_tools_steps;
mod config_steps;
mod context_pruning_steps;
mod cron_steps;
mod e2e_steps;
mod gateway_steps;
mod heartbeat_steps;
mod nsjail_steps;
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
mod wasm_steps;
mod wasm_tools_steps;

// Runner
// ===========================================================================

fn main() {
    let real_llm_enabled = std::env::var("QUECTO_REAL_LLM").unwrap_or_default() == "1";
    // Optional tag filter: QUECTO_TAG=real-llm runs only scenarios with that tag.
    let tag_filter = std::env::var("QUECTO_TAG").ok();
    // Optional deterministic scenario sharding across separate bdd processes.
    // Example: QUECTO_BDD_SHARD_INDEX=0 QUECTO_BDD_SHARD_TOTAL=4
    let shard_index = std::env::var("QUECTO_BDD_SHARD_INDEX")
        .ok()
        .and_then(|v| v.parse::<u64>().ok());
    let shard_total = std::env::var("QUECTO_BDD_SHARD_TOTAL")
        .ok()
        .and_then(|v| v.parse::<u64>().ok());
    let shard = match (shard_index, shard_total) {
        (Some(i), Some(t)) if t > 0 && i < t => Some((i, t)),
        _ => None,
    };
    let shard_plan = shard.map(|(_, total)| build_shard_plan("tests/features", total));

    futures::executor::block_on(
        QuectoWorld::cucumber()
            .max_concurrent_scenarios(25)
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
                // If a tag filter is set, require matching scenarios, but still
                // allow optional sharding to apply.
                if let Some(ref tag) = tag_filter {
                    let matches_feature = feat.tags.iter().any(|t| t == tag.as_str());
                    let matches_scenario = sc.tags.iter().any(|t| t == tag.as_str());
                    if !matches_feature && !matches_scenario {
                        return false;
                    }
                }
                // Optional process-level deterministic shard filter.
                if let Some((idx, total)) = shard {
                    if let Some(plan) = shard_plan.as_ref() {
                        let key = (feat.name.clone(), sc.name.clone());
                        if let Some(assigned) = plan.get(&key) {
                            if *assigned != idx {
                                return false;
                            }
                        } else if stable_hash(&feat.name, &sc.name) % total != idx {
                            return false;
                        }
                    } else if stable_hash(&feat.name, &sc.name) % total != idx {
                        return false;
                    }
                }
                // Include if feature or scenario is tagged @wip or @done
                feat.tags.iter().any(|t| t == "wip" || t == "done")
                    || sc.tags.iter().any(|t| t == "wip" || t == "done")
            }),
    );
}
