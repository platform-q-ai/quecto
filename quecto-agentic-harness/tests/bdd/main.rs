#![allow(private_interfaces)]

use cucumber::{World, gherkin, given, then, when};
use quecto::application::agent_loop::AgentLoopImpl;
use quecto::application::subagent::{SubagentConfig, SubagentContext, validate_agent_id};
use quecto::domain::agent::{AgentInfo, AgentLoop, AgentResult};
use quecto::domain::error::DomainError;
use quecto::domain::message::{LlmResponse, Message, Role, ToolCall};
use quecto::domain::provider::{ChatRequest, LlmProvider};
use quecto::domain::session::{ContextSpillStore, Session, SessionStore};
use quecto::domain::tool::{Tool, ToolDefinition, ToolResult};
use quecto::infrastructure::auth::credential_store::{
    AuthMethod, Credential, CredentialStatus, CredentialStore,
};
use quecto::infrastructure::config::Config;

use quecto::domain::provider_error::{ProviderErrorClass, classify_provider_error};
use quecto::infrastructure::persistence::session_store::FileSessionStore;
use quecto::infrastructure::providers;
use quecto::infrastructure::providers::router::ProviderRouter;

use quecto::infrastructure::security::sandbox::Sandbox;
use quecto::infrastructure::tools::bash::ExecTool;
use quecto::infrastructure::tools::registry::ToolRegistryImpl;
use quecto::infrastructure::tools::spawn::SpawnTool;
use quecto::infrastructure::tools::web_search::WebSearchTool;
use quecto::interface::cli::{self, CliContext};
use std::collections::HashMap;
use std::fs;
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

#[path = "../common/mod.rs"]
mod common;
mod feature_preprocess;

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
                    stop_reason: None,
                    thinking_blocks: vec![],
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
                name: name.to_string().into(),
                description: format!("Mock {} tool", name).into(),
                parameters_schema: r#"{"type":"object","properties":{}}"#.into(),
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
                image_blocks: vec![],
            })
        })
    }
}

// Wrapper for Arc<dyn Extension> that implements Debug (opaque).
pub struct DebugExtension(pub std::sync::Arc<dyn quecto::domain::extension::Extension>);

impl std::fmt::Debug for DebugExtension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<Extension>")
    }
}

impl Default for DebugExtension {
    fn default() -> Self {
        struct NullExt;
        impl quecto::domain::extension::Extension for NullExt {
            fn name(&self) -> &str {
                ""
            }
            fn tools(&self) -> Vec<std::sync::Arc<dyn quecto::domain::tool::Tool>> {
                vec![]
            }
        }
        Self(std::sync::Arc::new(NullExt))
    }
}

impl std::ops::Deref for DebugExtension {
    type Target = std::sync::Arc<dyn quecto::domain::extension::Extension>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

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

// Opaque Debug wrapper for the headless TUI render harness (#805). The harness
// holds a live `App` (and background tokio tasks) and isn't `Debug`, so wrap it
// to satisfy the derived `Debug`/`Default` on `QuectoWorld`.
pub struct TuiParityHarness(pub quecto_tui::interface::app::tui_harness::TuiHarness);

impl std::fmt::Debug for TuiParityHarness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<TuiParityHarness>")
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
    /// Stage B event-identity steps: emitting agent's id/parent and last event.
    pub event_identity_agent_id: Option<String>,
    pub event_identity_parent_id: Option<String>,
    pub event_identity_last: Option<serde_json::Value>,
    pub event_identity_stream: Vec<serde_json::Value>,
    /// Output of the `docs` tool under test (PRD docs-access work)
    pub docs_output: String,
    pub docs_is_error: bool,
    /// Loaded config (after "When I load the config")
    pub config: Option<Config>,
    /// Resolved workspace path (after "When I resolve the workspace path")
    pub resolved_workspace: Option<String>,
    /// Environment variable overrides to apply during config loading
    pub env_overrides: HashMap<String, String>,
    /// CLI context (allows overriding base_dir for tests)
    pub cli_context: CliContext,
    /// Security sandbox for testing path/command validation
    pub sandbox: Option<Sandbox>,
    /// Result of the last sandbox validation (Ok or Err message)
    pub validation_result: Option<Result<(), String>>,
    /// Tool registry for agent_tools scenarios
    pub tool_registry: Option<ToolRegistryImpl>,
    /// Temp dir for tool guard scenarios (kept alive)
    pub _tool_guard_tmp: Option<TempDir>,
    /// Path to the tool workspace (for file assertions)
    pub tool_workspace: Option<PathBuf>,
    /// Temp dir for tool workspace (kept alive)
    pub _tool_workspace_tmp: Option<TempDir>,
    /// Result of the last tool execution
    pub tool_result: Option<Result<ToolResult, String>>,
    /// Created LLM provider
    pub provider: Option<Arc<dyn LlmProvider>>,
    /// Error classification result
    pub error_class: Option<ProviderErrorClass>,
    /// Fallback provider for fallback/cooldown scenarios
    pub provider_router: Option<Arc<ProviderRouter>>,
    /// Response from fallback provider
    pub router_response: Option<LlmResponse>,
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
    /// Session keys created during routing scenarios
    pub session_keys: HashMap<String, String>,
    /// Credential store for auth scenarios
    pub credential_store: Option<CredentialStore>,
    /// Auth status summary from the last check
    pub auth_status: Option<Vec<CredentialStatus>>,
    /// Spawn tool result
    pub spawn_result: Option<ToolResult>,
    /// Spawn tool for BDD
    pub spawn_tool: Option<SpawnTool>,
    /// AgentCmdTool for BDD (#421)
    pub agent_cmd_tool: Option<quecto::infrastructure::tools::agent_cmd::AgentCmdTool>,
    /// AgentCmdTool result for BDD (#421)
    pub agent_cmd_result: Option<ToolResult>,
    /// Shared subagent registry for BDD (#421)
    pub agent_cmd_registry: Option<quecto::infrastructure::tools::agent_cmd::SubagentRegistry>,
    /// Mock UDS server temp dir for agent_cmd BDD (kept alive)
    pub _agent_cmd_mock_tmp: Option<TempDir>,
    /// Last command sent to mock UDS server (#421)
    pub agent_cmd_last_command: Option<Arc<Mutex<String>>>,
    /// Subagent spawn config for subagent scenarios
    pub subagent_config: Option<SubagentConfig>,
    /// Created subagent context
    pub subagent_context: Option<SubagentContext>,
    /// Agent allowlist for subagent validation scenarios
    pub agent_allowlist: Vec<String>,
    /// Result of agent_id validation
    pub agent_id_validation: Option<Result<(), String>>,
    /// Wiremock server URI (kept alive via Box leak)
    pub _wiremock_server_uri: Option<String>,
    /// Provider protocol currently backed by the generic mock provider helpers.
    pub mock_provider_kind: Option<String>,
    /// True when a retired live behavioral scenario is running in the @mock-llm lane.
    pub auto_mock_manual_llm: bool,
    /// Fireworks-compatible mock server URI for provider reload scenarios
    pub _fireworks_mock_uri: Option<String>,
    /// Leaked Fireworks-compatible mock server ref for request inspection
    pub fireworks_mock_server_ref: Option<&'static wiremock::MockServer>,
    /// Leaked wiremock server ref for request inspection
    pub wiremock_server_ref: Option<&'static wiremock::MockServer>,
    /// Temp directory handle (kept alive so the dir isn't deleted)
    pub _temp_dir: Option<TempDir>,
    /// Additional temp dirs (kept alive for sandbox hardening symlink tests etc.)
    pub _extra_temp_dirs: Vec<TempDir>,
    /// Exec tool for direct exec tool testing (timeout, env sanitization)
    pub exec_tool: Option<Arc<ExecTool>>,
    /// Environment variable overrides for exec tool env sanitization tests
    pub exec_env_vars: HashMap<String, String>,
    /// TUI scrollback BDD: chat view under test.
    pub tui_chat: Option<quecto_tui::interface::components::chat::Chat>,
    /// TUI footer BDD (#760): footer render while marked as streaming.
    pub tui_footer_streaming_render: Vec<String>,
    /// TUI footer BDD (#760): footer render while idle (not streaming).
    pub tui_footer_idle_render: Vec<String>,
    /// TUI @files BDD: file-mention autocomplete under test.
    pub tui_files_autocomplete:
        Option<quecto_tui::interface::components::files_autocomplete::FilesAutocomplete>,
    /// TUI @files BDD: last consumed background-load request.
    pub tui_files_load_requested: bool,
    /// TUI sub-agent session-parity BDD (#805): tokio runtime backing the
    /// headless render harness (its background tasks need a live runtime).
    pub tui_parity_rt: Option<tokio::runtime::Runtime>,
    /// TUI sub-agent session-parity BDD (#805): the headless render harness.
    pub tui_parity: Option<TuiParityHarness>,
    /// The sub-agent id currently being viewed (#828): captured on select so
    /// backfill/assertion steps route to the right session, not a literal id.
    pub tui_viewed_agent: Option<String>,
    /// TUI scrollback BDD: viewport captured before streaming growth.
    pub tui_viewport_before_stream: Vec<String>,
    /// TUI scrollback BDD: viewport captured after streaming growth.
    pub tui_viewport_after_stream: Vec<String>,
    /// Provider wiring: resolved API key for a provider
    pub gateway_resolved_api_key: Option<String>,
    /// Provider readiness report
    pub gateway_readiness_report: Option<Vec<String>>,
    /// Config for provider wiring tests
    pub gateway_config: Option<Config>,
    /// Credential store for wiring tests
    pub gateway_credential_store: Option<CredentialStore>,
    /// Credential snapshot (loaded once, shared across resolution steps)
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
    /// UDS agent: accumulated command lines to send (built up by "I send" steps)
    pub uds_commands: Vec<String>,
    /// UDS agent: captured event lines after execution
    pub agent_events: Vec<String>,
    /// UDS agent: captured execution stderr from the helper thread
    pub uds_execution_error: Option<String>,
    /// UDS agent: replace config with invalid JSON after building the agent but before the command loop
    pub uds_invalid_config_before_loop: bool,
    /// UDS agent: add Fireworks provider after building the agent but before the command loop
    pub uds_add_fireworks_before_loop: bool,
    /// UDS agent: captured stderr after execution
    pub agent_stderr: String,
    /// UDS agent: exit code after execution
    pub uds_exit_code: Option<i32>,
    /// UDS agent: session name flag (None = no session, Some(name) = named, Some("-") = ephemeral)
    pub session_name: Option<String>,
    /// UDS agent: use --no-session flag
    pub no_session: bool,
    /// UDS agent: optional system prompt from --system flag
    pub system_prompt: Option<String>,
    /// UDS agent: path to the socket file used in the last execute_uds() run
    pub _uds_socket_path: Option<std::path::PathBuf>,
    /// When true, the UDS agent builder enables incremental streaming.
    pub _uds_streaming_enabled: bool,
    /// UDS agent: when true, pass an explicit socket path to run_uds_loop
    pub _uds_use_explicit_socket: bool,
    /// UDS agent: path used when testing the real bind path (socket_override = None)
    pub _uds_real_bind_socket_path: Option<std::path::PathBuf>,
    /// UDS agent: unix mode bits sampled from the socket file after bind
    pub _uds_real_bind_socket_mode: Option<u32>,
    /// REPL: accumulated input lines (built up by "I type" steps)
    pub repl_input_lines: Vec<String>,
    /// REPL: flags to pass (built up by "with flags" steps)
    pub repl_flags: Vec<String>,
    /// REPL: whether the REPL has been executed (lazy execution)
    pub repl_executed: bool,
    /// REPL: captured progress event labels (for progress recorder scenarios)
    pub repl_progress_events: Vec<String>,
    /// REPL: whether to inject a progress recorder callback
    pub repl_use_progress_recorder: bool,
    /// REPL: whether to force TTY mode (for TTY-specific rendering tests)
    pub repl_force_tty: bool,
    /// Leaked wiremock server ref for web search mock (for mounting responses)
    pub web_search_mock_server: Option<&'static wiremock::MockServer>,
    /// Whether the web search used DDG (for fallback assertion)
    pub web_search_used_ddg: bool,
    /// Leaked wiremock server ref for web_fetch mock
    pub _web_fetch_mock_server: Option<&'static wiremock::MockServer>,
    /// Mock server URI for web_fetch tool
    pub _web_fetch_mock_uri: Option<String>,
    /// Pending CLI args for interactive auth scenarios (set by "I start quecto")
    pub pending_cli_args: Option<Vec<String>>,
    /// Captured tracing log output for observability scenarios
    pub captured_log_output: Option<Arc<Mutex<String>>>,
    /// Streaming response from provider streaming scenarios
    pub streaming_response: Option<LlmResponse>,
    /// Incremental stream events collected from chat_stream_incremental scenarios (#181)
    pub stream_events: Vec<quecto::domain::provider::StreamEvent>,
    /// Whether any parse error occurred during incremental streaming (#181)
    pub stream_had_parse_error: bool,
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
    // --- Path utils BDD fields ---
    /// Resolved path from path_utils scenarios
    pub path_utils_resolved: Option<std::path::PathBuf>,
    // --- Truncation BDD fields ---
    /// Input string for truncation scenarios
    pub truncation_input: Option<String>,
    /// Result from truncate_head / truncate_tail
    pub truncation_result: Option<quecto::infrastructure::tools::truncate::TruncationResult>,
    /// Result from truncate_line
    pub truncation_line_result: Option<(String, bool)>,
    // --- Grep BDD fields ---
    /// Temp dir for grep workspace (kept alive)
    pub _grep_temp_dir: Option<TempDir>,
    /// Workspace path for grep tests
    pub grep_workspace: Option<PathBuf>,
    /// Result from grep tool execution
    pub grep_result: Option<quecto::domain::tool::ToolResult>,
    // --- Find BDD fields ---
    /// Temp dir for find workspace (kept alive)
    pub _find_temp_dir: Option<TempDir>,
    /// Workspace path for find tests
    pub find_workspace: Option<PathBuf>,
    /// Result from find tool execution
    pub find_result: Option<quecto::domain::tool::ToolResult>,
    // --- Ls BDD fields ---
    /// Temp dir for ls workspace (kept alive)
    pub _ls_temp_dir: Option<TempDir>,
    /// Workspace path for ls tests
    pub ls_workspace: Option<PathBuf>,
    /// Result from ls tool execution
    pub ls_result: Option<quecto::domain::tool::ToolResult>,
    // --- ensure_tool BDD fields ---
    /// Temp dir for ensure_tool cache (kept alive)
    pub _ensure_tool_tmp: Option<TempDir>,
    /// Result path from ensure_tool (Ok)
    pub ensure_tool_result: Option<Result<std::path::PathBuf, String>>,
    // --- /reload BDD fields ---
    /// Messages passed to strip_tool_history in /reload scenarios
    pub reload_input_messages: Option<Vec<Message>>,
    /// Filtered messages returned by strip_tool_history
    pub reload_filtered_messages: Option<Vec<Message>>,
    /// Response text from /reload handler
    pub reload_response: Option<String>,
    /// Session store used in /reload scenarios
    pub reload_session_store:
        Option<Arc<quecto::infrastructure::persistence::session_store::FileSessionStore>>,
    /// Spill store used in /reload scenarios
    pub reload_spill_store:
        Option<Arc<quecto::infrastructure::persistence::context_spill::FileContextSpillStore>>,
    /// Temp dir for /reload scenarios (kept alive)
    pub _reload_temp_dir: Option<TempDir>,
    /// Stored command for multi-step exec scenarios (e.g. large output tests)
    pub stored_command: Option<String>,
    /// Model routing: name of the provider that actually handled the request
    pub routing_handled_by: Option<String>,
    /// Model routing: whether the routing request succeeded
    pub routing_succeeded: Option<bool>,
    /// Model routing: response content from routing scenario
    pub routing_response: Option<LlmResponse>,
    // --- Issue #182: cancellation support ---
    /// Cancel flag for cancellation BDD scenarios
    pub cancel_flag: Option<quecto::domain::provider::CancelFlag>,
    /// Result of a chat() call (Ok(response) or Err(message)) for cancellation tests
    pub chat_result: Option<Result<LlmResponse, String>>,
    /// Result of a chat_stream() call for cancellation tests
    pub chat_stream_result: Option<Result<LlmResponse, String>>,
    /// Parsed stop reason for stop_reason parsing tests
    pub parsed_stop_reason: Option<quecto::domain::message::StopReason>,
    /// Normalized API messages (for normalization scenario assertions)
    pub api_messages: Vec<serde_json::Value>,
    /// Mock OAuth refresh server URI (for OAuth refresh scenarios, issue #254)
    pub gateway_oauth_mock_uri: Option<String>,
    /// Leaked wiremock server ref for OAuth refresh mock (kept alive)
    pub _gateway_oauth_mock_server: Option<&'static wiremock::MockServer>,
    /// Token exchange result (for issue #257 scenarios)
    pub gateway_token_exchange_result: Option<
        Result<
            quecto::infrastructure::auth::oauth::OAuthTokenResponse,
            quecto::domain::error::DomainError,
        >,
    >,
    /// OAuth expires_in value for margin tests (issue #256)
    pub gateway_expires_in: Option<u64>,
    /// Computed expires_at for margin assertions (issue #256)
    pub gateway_computed_expires_at: Option<i64>,
    /// `now` captured at the moment expires_at was computed, so the assertion
    /// is race-free under cucumber's concurrent scenario execution (the old
    /// code recomputed `now` in the Then step, which could drift seconds past
    /// the tolerance under CPU contention).
    pub gateway_expires_at_reference_now: Option<i64>,
    /// Auth.json for import scenarios (issue #258)
    pub gateway_import_auth_json: Option<serde_json::Value>,
    /// Import stdout output (issue #258)
    pub gateway_import_stdout: Option<String>,
    /// Import stderr output (issue #258)
    pub gateway_import_stderr: Option<String>,
    /// RefreshableProvider result (issue #255)
    pub refreshable_result: Option<Result<LlmResponse, DomainError>>,
    /// RetryingProvider scenarios (#931): inner counting provider.
    pub retry_inner: Option<Arc<dyn LlmProvider>>,
    /// RetryingProvider scenarios (#931): shared inner-call counter.
    pub retry_call_count: Option<Arc<std::sync::atomic::AtomicU32>>,
    /// RetryingProvider scenarios (#931): max attempts for the decorator.
    pub retry_max_attempts: Option<u32>,
    /// RetryingProvider scenarios (#931): whether the decorated call succeeded.
    pub retry_succeeded: Option<bool>,
    // --- Workflow V2 (UDS-only, #568–#577) ---
    // V1 BDD workflow fields removed. V2 workflow is covered by unit tests in:
    //   src/domain/workflow_tests.rs
    //   src/infrastructure/tools/workflow_tool_comprehensive_tests.rs
    //   src/interface/shared_tests.rs
    //   src/interface/cli/agent_tests.rs
    //   src/interface/cli/protocol_tests.rs
    //   src/infrastructure/persistence/session_store.rs
    // --- Extension system BDD fields ---
    /// Test extension for extension trait scenarios (Debug-opaque)
    pub test_extension: Option<DebugExtension>,
    /// Extension registry for extension registry scenarios
    pub ext_registry: Option<quecto::infrastructure::extensions::registry::ExtensionRegistry>,

    /// Native extension under test (Debug-opaque)
    pub native_extension: Option<DebugExtension>,
    /// Built native extensions from config (Debug-opaque)
    pub native_extensions_built: Option<Vec<DebugExtension>>,
    /// Custom config path for --config flag scenarios
    pub custom_config_path: Option<String>,
    /// Token estimation: input string for estimate_tokens scenarios
    pub token_estimate_input: Option<String>,
    /// Guard check result
    pub guard_check_result: Option<Result<(), String>>,
    /// Captured guard tool name
    pub guard_captured_name: Option<String>,
    /// Captured guard arguments
    pub guard_captured_args: Option<String>,
    // --- Subagent notify (#523) ---
    /// Formatted notification message for BDD assertions
    pub notify_message: Option<String>,
    /// JSON messages array for summary extraction scenarios
    pub notify_messages_json: Option<serde_json::Value>,
    /// Extracted summary from agent_end messages
    pub notify_extracted_summary: Option<String>,
    /// Notification channel sender for BDD
    pub notify_tx: Option<quecto::infrastructure::tools::subagent_registry::NotificationTx>,
    /// Notification channel receiver for BDD
    pub notify_rx: Option<quecto::infrastructure::tools::subagent_registry::NotificationRx>,
    /// Count from drain operation
    pub notify_drain_count: Option<usize>,
    /// Parent session under test for #816 auto-await idle delivery
    pub notify_parent_session: Option<quecto::interface::cli::uds_session::AgentSession>,
    /// Per-agent completion sequence counter (kept out of the Gherkin, #816)
    pub notify_seq: std::collections::HashMap<String, u64>,
    /// Result of the most recent enqueue (true=delivered, false=ignored) (#816)
    pub notify_last_enqueued: Option<bool>,
    /// The parent's first drained idle note, cached so multiple assertions in a
    /// scenario inspect the same note rather than re-draining the queue (#816)
    pub notify_drained_note: Option<quecto::domain::message::Message>,
    // --- Subagent monitor (#522) ---
    /// SubagentEntry under test for monitor BDD scenarios
    pub monitor_entry: Option<quecto::infrastructure::tools::subagent_registry::SubagentEntry>,
    /// All SubagentStatus variants for display assertion
    pub monitor_status_variants:
        Option<Vec<quecto::infrastructure::tools::subagent_registry::SubagentStatus>>,
    /// Root registry for cascade-remove BDD scenarios (#831)
    pub cascade_registry:
        Option<quecto::infrastructure::tools::subagent_registry::SubagentRegistry>,
    /// Result of a cascade-remove broadcast: Some(event) or None (#831)
    pub cascade_broadcast: Option<Option<serde_json::Value>>,
    /// Monitor abort handle for cancellation test
    pub monitor_abort_handle: Option<tokio::task::JoinHandle<()>>,
    /// Tokio runtime for abort handle test (keeps spawned task alive)
    pub _monitor_rt: Option<tokio::runtime::Runtime>,
    /// Result of aborting the monitor handle
    pub monitor_abort_result: Option<bool>,
    // --- Subagent protocol (#524) ---
    /// SubagentInfo list from get_subagents
    pub subagent_infos: Vec<quecto::interface::cli::protocol::SubagentInfo>,
    /// Single SubagentInfo under test
    pub subagent_info_single: Option<quecto::interface::cli::protocol::SubagentInfo>,
    /// Serialized SubagentInfo JSON
    pub subagent_info_json: serde_json::Value,
    /// Raw JSON command string for parsing
    pub protocol_command_json: String,
    /// Parsed command
    pub parsed_command: Option<quecto::interface::cli::protocol::AgentCommand>,
    /// Protocol event under test
    pub protocol_event: Option<quecto::interface::cli::protocol::AgentEvent>,
    /// Deserialized event for round-trip test
    pub deserialized_event: Option<quecto::interface::cli::protocol::AgentEvent>,
    /// Registry for protocol BDD scenarios
    pub subagent_protocol_registry:
        Option<quecto::infrastructure::tools::subagent_registry::SubagentRegistry>,
    // --- Subagent widget (#525) ---
    /// Simulated subagent infos for widget BDD tests
    pub widget_subagent_infos: Vec<quecto::interface::cli::protocol::SubagentInfo>,
    /// Rendered widget lines
    pub widget_bar_lines: Vec<String>,
    // --- Multi-client UDS (#318) ---
    /// Multi-client UDS: per-client command queues (client_id -> commands)
    pub mc_client_commands: HashMap<u32, Vec<String>>,
    /// Multi-client UDS: per-client received events (client_id -> event lines)
    pub mc_client_events: HashMap<u32, Vec<String>>,
    /// Multi-client UDS: set of connected client IDs
    pub mc_connected_clients: Vec<u32>,
    /// Multi-client UDS: set of client IDs that explicitly disconnected
    pub mc_disconnected_clients: Vec<u32>,
    /// Multi-client UDS: whether multi-client mode is requested
    pub mc_mode: bool,
    /// Multi-client UDS: exit code from multi-client execution
    pub mc_exit_code: Option<i32>,
    /// Multi-client UDS: whether hot-reload watcher should be enabled

    /// Multi-client UDS: extensions to create after agent starts (for hot-reload tests)

    /// Multi-client UDS: when true, start with --persist flag (#348)
    pub _mc_persist: bool,
    /// Multi-client UDS: per-client tool_name → reply content for
    /// `client N replies to execute_tool` reactive step. The harness
    /// watches the stream for an `execute_tool` event with a matching
    /// toolName and auto-sends a `tool_result` carrying the content.
    pub mc_auto_replies: HashMap<u32, Vec<(String, String)>>,
    /// Multi-client UDS: clients to connect after all others have disconnected (#348)
    pub _mc_reconnect_clients: Vec<u32>,
    /// Real-LLM UDS mode: use real credentials and real socket bind with sequential prompts
    pub _real_llm_uds: bool,
    /// Workflow V2: when true, register WorkflowEngine + WorkflowTool + WorkflowGuard
    pub _workflow_enabled: bool,
    // --- Audit log (#609) ---
    /// Temp dir for audit log tests (kept alive)
    pub tempdir: Option<TempDir>,
    /// Audit event under test (for serde round-trip scenarios)
    pub audit_event: Option<quecto::domain::audit::AuditEvent>,
    /// Serialized JSON for audit event
    pub audit_json: Option<String>,
    /// Active audit log handle
    pub audit_log: Option<std::sync::Arc<quecto::infrastructure::persistence::audit_log::AuditLog>>,
    /// Session key used with audit log
    pub audit_session_key: Option<String>,
    /// Long content for preview tests
    pub audit_long_content: Option<String>,
    /// Generated content preview
    pub audit_content_preview: Option<String>,
    // --- agent_cmd await (#612) ---
    /// Parsed await result for BDD assertions
    pub await_result: Option<serde_json::Value>,
    /// Mock await registry for BDD scenarios
    pub await_registry: Option<quecto::infrastructure::tools::agent_cmd::SubagentRegistry>,
    /// Active awaits tracker for BDD scenarios
    pub await_active_awaits: Option<quecto::infrastructure::tools::agent_cmd::ActiveAwaits>,
    /// Temp dir for await mock sockets (kept alive)
    pub _await_mock_tmp: Option<TempDir>,
    /// Mock listener for await scenarios (kept alive)
    pub _await_mock_listener: Option<std::os::unix::net::UnixListener>,
    /// RuntimeReload BDD: temp dir holding the watched source file(s)
    pub _reload_tmp: Option<TempDir>,
    /// RuntimeReload BDD: path → file label map (for multi-source scenarios)
    pub reload_files: HashMap<String, PathBuf>,
    /// RuntimeReload BDD: the reload gate under test (string last-good)
    pub reload_gate: Option<quecto::infrastructure::reload::RuntimeReload<String>>,
    /// RuntimeReload BDD: single reload source under test
    pub reload_source: Option<quecto::infrastructure::reload::ReloadSource>,
    /// RuntimeReload BDD: captured mtime before a touch, for cache-advance asserts
    pub reload_mtime_before: Option<std::time::SystemTime>,
    /// RuntimeReload BDD: whether the rebuild closure was invoked
    pub reload_rebuild_called: Arc<Mutex<bool>>,
    /// RuntimeReload BDD: result of the last poll/force-poll
    pub reload_poll_result: Option<quecto::infrastructure::reload::ReloadResult<String>>,
    /// RuntimeReload BDD: result of the last source probe
    pub reload_source_change: Option<quecto::infrastructure::reload::SourceChange>,
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

fn scenario_weight(tags: &[String], step_lines: &[String], _feature: &str, _scenario: &str) -> u64 {
    let mut w = 1_u64;
    if tags.iter().any(|t| t == "manual-real-llm") {
        w += 4;
    }
    if tags.iter().any(|t| t == "real-llm-smoke") {
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

fn dotenv_value(var: &str) -> Option<String> {
    let contents = std::fs::read_to_string(".env").ok()?;
    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        if let Some(value) = line.strip_prefix(&format!("{var}=")) {
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn any_env_or_dotenv(vars: &[&str]) -> bool {
    vars.iter().any(|var| {
        std::env::var(var).is_ok_and(|value| !value.trim().is_empty())
            || dotenv_value(var).is_some()
    })
}

fn default_quecto_base_dir() -> PathBuf {
    std::env::var("QUECTO_BASE_DIR")
        .ok()
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".quecto")))
        .unwrap_or_else(|| PathBuf::from(".quecto"))
}

fn has_openai_oauth_credential() -> bool {
    let store = CredentialStore::new(default_quecto_base_dir());
    store
        .get("openai")
        .ok()
        .flatten()
        .is_some_and(|credential| credential.method == AuthMethod::OAuth)
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
        .map(|row| {
            let key = row[0].trim().to_string();
            let raw = row[1].trim();
            // Coerce numeric-looking values to JSON numbers so tool handlers can use
            // as_u64()/as_i64(). NOTE: string-only table values (e.g. paths) are not
            // affected since paths never parse as i64. If a future test needs a string
            // that looks numeric (e.g. "10"), use a Gherkin docstring instead of a table.
            let val = if let Ok(n) = raw.parse::<i64>() {
                serde_json::json!(n)
            } else if let Ok(f) = raw.parse::<f64>() {
                serde_json::json!(f)
            } else {
                serde_json::json!(raw)
            };
            (key, val)
        })
        .collect::<serde_json::Map<String, serde_json::Value>>()
        .into();
    obj.to_string()
}

mod agent_cmd_await_steps;
mod agent_cmd_tool_steps;
mod agent_loop_steps;
mod agent_tools_steps;
mod architecture_steps;
mod audit_log_steps;
mod auth_steps;
mod codex_provider_steps;
mod config_steps;
mod context_pruning_steps;
mod e2e_steps;
mod edit_tool_steps;
mod embedded_docs_steps;
mod ensure_tool_steps;
mod exec_tool_steps;
mod extension_steps;
mod find_steps;
mod grep_steps;
mod ls_steps;
mod mouse_selection_steps;
mod observability_steps;
mod path_utils_steps;
mod provider_steps;
mod read_tool_steps;
mod release_profile_steps;
mod reload_steps;
mod repl_steps;
mod repo_docs_steps;
mod sandbox_steps;
mod security_steps;
mod session_steps;
mod spawn_tool_steps;
mod subagent_bar_fixes_steps;
mod subagent_monitor_steps;
mod subagent_notify_steps;
mod subagent_protocol_steps;
mod subagent_steps;
mod subagent_widget_steps;
mod tool_empty_args_steps;
mod truncate_steps;
mod tui_architecture_steps;
mod tui_cold_start_steps;
mod tui_file_mention_steps;
mod tui_subagent_first_layout_steps;
mod tui_subagent_parity_steps;
mod uds_steps;
mod web_fetch_steps;
mod workflow_event_identity_steps;

// Runner
// ===========================================================================

fn main() {
    let real_llm_enabled = std::env::var("QUECTO_REAL_LLM").unwrap_or_default() == "1";
    let provider_smoke_enabled = std::env::var("QUECTO_PROVIDER_SMOKE").unwrap_or_default() == "1";
    // Optional tag filter: QUECTO_TAG=manual-real-llm runs only the retired live suite.
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

    // Strip chainlink-style `[noun]` step tags into a tempdir before cucumber
    // parses the features. On-disk .feature files keep their brackets (so
    // `chainlink scan` sees the authoritative noun-feature mapping) while
    // cucumber-rs sees the bare-prose form its step-def regexes are written
    // against. Tempdir must outlive the cucumber run, hence the binding here.
    let (_stripped_dir, stripped_features_path) =
        feature_preprocess::stripped_features_tempdir(std::path::Path::new("tests/features"))
            .expect("failed to preprocess .feature files into tempdir");

    futures::executor::block_on(
        QuectoWorld::cucumber()
            .max_concurrent_scenarios(25)
            .fail_on_skipped()
            // `_and_exit` makes the process exit non-zero when any scenario
            // fails. Plain `filter_run` returns normally even on failure, so
            // the bdd test binary exited 0 with failing scenarios — meaning the
            // pre-push BDD shards could never fail the gate. (This is how 38
            // deterministic scenarios stayed red on master undetected.)
            .filter_run_and_exit(stripped_features_path.clone(), move |feat, _, sc| {
                // Exclude scenarios explicitly tagged @pending
                if sc.tags.iter().any(|t| t == "pending") {
                    return false;
                }
                // Exclude the retired live behavioral suite unless it is either
                // explicitly enabled for live credentials or selected through
                // the zero-cost @mock-llm mirror lane.
                if sc.tags.iter().any(|t| t == "manual-real-llm")
                    && !real_llm_enabled
                    && tag_filter.as_deref() != Some("mock-llm")
                {
                    return false;
                }
                // Exclude live provider smoke scenarios unless explicitly enabled.
                if sc.tags.iter().any(|t| t == "provider-smoke") && !provider_smoke_enabled {
                    return false;
                }
                // Provider-specific smoke credentials are optional: filter absent
                // providers rather than failing unrelated smoke scenarios.
                if provider_smoke_enabled {
                    if sc.tags.iter().any(|t| t == "provider-smoke-openai")
                        && !any_env_or_dotenv(&["OPENAI_API_KEY"])
                    {
                        return false;
                    }
                    if sc.tags.iter().any(|t| t == "provider-smoke-anthropic")
                        && !any_env_or_dotenv(&["ANTHROPIC_API_KEY"])
                    {
                        return false;
                    }
                    if sc.tags.iter().any(|t| t == "provider-smoke-codex")
                        && !has_openai_oauth_credential()
                    {
                        return false;
                    }
                }
                // Partition the zero-cost mocked e2e lane from the default wave.
                // @mock-llm scenarios are also tagged @done, so the untagged
                // pre-push wave (step 5) would otherwise run them — and the
                // dedicated mock lane (step 9, QUECTO_TAG=mock-llm) re-runs them,
                // executing the mock suite twice per push. Only include @mock-llm
                // when the mock lane explicitly selects it.
                let is_mock_llm = feat.tags.iter().any(|t| t == "mock-llm")
                    || sc.tags.iter().any(|t| t == "mock-llm");
                if is_mock_llm && tag_filter.as_deref() != Some("mock-llm") {
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
