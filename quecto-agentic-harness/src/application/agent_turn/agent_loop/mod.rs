mod request;
mod run_loop;
use crate::application::agent_turn::agent_loop::agent_loop_policy::ToolPolicyState;
pub use crate::application::agent_turn::agent_loop::agent_usage::UsageTotals;
use crate::application::agent_turn::agent_loop::durable_prefix::DurablePrefixLatch;
use crate::application::agent_turn::context::context_pruning;
use crate::application::agent_turn::context::{ContextManager, ContextManagerConfig};
use crate::application::agent_turn::ports::AgentLoop;
use crate::application::audit::ports::AuditSink;
use crate::application::providers::ports::LlmProvider;
use crate::application::tools::ports::{
    RuntimeToolLifecycleRegistry, SessionAwareTools, ToolCatalog, ToolExecutor, ToolRegistry,
};
use crate::domain::agents::value_objects::agent::{AgentInfo, AgentProgressEvent};
use crate::domain::agents::value_objects::agent::{AgentResult, ProgressCallback};
use crate::domain::conversation::services::reply_requirement::ReplyRequirement;
use crate::domain::conversation::services::watermark::Watermark;
use crate::domain::conversation::value_objects::message::{LlmResponse, Message, ToolCall};
use crate::domain::inference::events::request_observation::{
    InputBaseline, RequestDiagnostics, RequestObservation,
};
use crate::domain::inference::services::provider_error::classify_provider_error;
use crate::domain::inference::value_objects::provider::{EffortLevel, StreamEvent};
use crate::domain::sessions::entities::session_identity::SessionIdentity;
use crate::domain::tool_policy::value_objects::tool;
use crate::domain::tool_policy::value_objects::tool_descriptor::ToolDescriptor;
use crate::domain::{audit::AuditEvent, shared::error::DomainError};
use agent_loop_stream::{
    StreamProviderError, TurnEnd, empty_stream_error_message, is_empty_streamed_response,
};
use std::{pin::Pin, sync::Arc};
use tool::{ToolDefinition, ToolPolicyReconciliation, ToolPolicyRequest, ToolProfileContext};
pub type ToolPolicyPersistence =
    Arc<dyn Fn(&ToolPolicyReconciliation) -> Result<(), String> + Send + Sync>;
mod agent_loop_clamp;
mod agent_loop_effort;
mod agent_loop_gauge;
mod agent_loop_lifecycle_compat;
mod agent_loop_policy;
pub(crate) mod agent_loop_preview;
mod agent_loop_pruning;
mod agent_loop_reload;
mod agent_loop_session;
mod agent_loop_spill;
mod agent_loop_stream;
#[cfg(any(test, feature = "test-support"))]
pub mod agent_loop_test_support;
mod agent_loop_tool_exec;
mod agent_loop_turn;
mod agent_loop_turn_flow;
mod agent_loop_uds_tools;
mod agent_usage;
pub mod durable_prefix;
mod request_observation;
pub mod tool_panic_scope;
use crate::domain::inference::services::terminal_guidance::Output;
use agent_loop_spill::ToolMessageArgs;
use agent_loop_turn::{TurnState, next_state_after_provider_response};
const DEFAULT_MAX_TOOL_ITERATIONS: u32 = 999_999;
const MAX_PROVIDER_ATTEMPTS: usize = 3;
const PROVIDER_RETRY_BACKOFF_MS: u64 = 100;
pub struct AgentLoopConfig {
    pub provider: Arc<dyn LlmProvider>,
    pub tool_registry: Box<dyn ToolRegistry>,
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
    /// The loop's retention handles (D9 #1978), composed over the one
    /// retention store; `None` retains nothing.
    pub retention: Option<crate::application::agent_turn::context::ContextRetention>,
    pub session_key: String,
    pub max_context_tokens: usize,
    /// Optional live progress events callback (REPL renderer); `None` = headless no-op.
    pub progress_callback: Option<ProgressCallback>,
    /// `true`: stream Token events live (UDS); `false` for REPL (non-streaming mocks).
    pub streaming: bool,
    /// Optional effort level for every `ChatRequest`; `None` = provider default.
    pub effort: Option<EffortLevel>,
    /// Optional append-only audit log. When `Some`, every significant event
    /// (tool call, tool result, LLM turn, pruning, etc.) is written to a
    /// durable JSONL file. When `None`, no audit overhead.
    pub audit_log: Option<Arc<dyn AuditSink>>,
    /// #1045: recent-turn tail-pin count for the emergency ladder. A
    /// constructor field (not a post-construction builder) so no construction
    /// site can silently drop the user's configured value.
    pub pin_recent_turns: u32,
    /// #2403/#2414: the watermark marks (cut at the high, down to the low),
    /// before the ceiling scales them. Constructor field for the same reason.
    pub context_marks: Watermark,
    /// #1044: active model context window (`None` unknown); bounds pruning budget.
    pub model_context_window: Option<usize>,
    pub tool_profile_context: ToolProfileContext,
}
pub struct AgentLoopImpl {
    unreported_usage: std::sync::Mutex<UsageTotals>,
    accounting_outbox: std::sync::Mutex<Vec<RequestObservation>>,
    request_observations: std::sync::Mutex<RequestDiagnostics>,
    /// The request in flight, which `get_state` reports (#2210).
    in_flight_request: Arc<crate::domain::inference::events::request_progress::InFlightRequest>,
    /// This agent's own provider requests so far, for `get_state` (#2436).
    request_tally: Arc<crate::domain::inference::events::request_completion::RequestTally>,
    /// Requests that ended in flight, awaiting their audit record (#2210).
    interrupted_requests: std::sync::Mutex<Vec<request_observation::InterruptedRequest>>,
    request_admission: Option<Arc<dyn crate::application::providers::ports::RequestAdmission>>,
    request_accounting: Option<Arc<dyn crate::application::providers::ports::RequestAccounting>>,
    tool_admission: Option<Arc<dyn crate::application::tools::ports::ToolExecutionAdmission>>,
    request_prefix: std::sync::Mutex<Option<String>>,
    /// The session's input baseline, which outlives a rebuilt provider (#2398).
    input_baseline: InputBaseline,
    provider: Arc<dyn LlmProvider>,
    pub(super) tool_registry: Box<dyn ToolRegistry>,
    model: String,
    max_tokens: u32,
    /// Per-model registry output cap, if known; see `agent_loop_clamp` (#935).
    model_max_tokens: Option<u32>,
    /// One request after an output-limit cut-off may go above the limit (#2124).
    output_boost: std::sync::atomic::AtomicBool,
    last_request_max_tokens: std::sync::atomic::AtomicU32,
    temperature: f32,
    max_tool_iterations: u32,
    /// Whether the loop retains context (D9 #1978): the spill manifest is
    /// refreshed after a tool turn only when it does.
    retains_context: bool,
    session_key: String,
    /// #1044: the active model's known context window (None when unknown).
    pub(super) model_context_window: Option<usize>,
    /// #2405, #2421: how the active model's prompt is bounded; if it takes images.
    model_traits: agent_loop_clamp::ModelTraits,
    /// #2405: the window note awaiting the next request, once per model.
    context_notes: std::sync::Mutex<agent_loop_clamp::ContextNotes>,
    /// When true, use incremental streaming for LLM calls.
    streaming: bool,
    /// Optional live progress callback wired by interactive agent clients.
    progress_callback: Option<ProgressCallback>,
    /// Optional effort level passed through to every ChatRequest.
    pub(super) effort: Option<EffortLevel>,
    /// Startup default effort, restored on session switches (#1067).
    pub(super) default_effort: Option<EffortLevel>,
    /// Optional append-only audit log for durable event recording.
    audit_log: Option<Arc<dyn AuditSink>>,
    /// #1072: latched by `apply_context_pruning` whenever a pass mutated
    /// existing history; the session save transaction consumes it.
    durable_prefix_dirty: Arc<DurablePrefixLatch>,
    /// Context-management boundary for pruning, spilling, dirty-prefix, and
    /// user-facing context gauge decisions.
    context_manager: ContextManager,
    pub(super) pending_tool_policy_requests: std::sync::Mutex<Vec<ToolPolicyRequest>>,
    pub(super) tool_policy_state: std::sync::Mutex<ToolPolicyState>,
    pub(super) turn_in_flight: std::sync::atomic::AtomicBool,
    pub(super) tool_profile_context: ToolProfileContext,
    pub(super) tool_policy_persistence: Option<ToolPolicyPersistence>,
}
impl std::fmt::Debug for AgentLoopImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentLoopImpl")
            .field("provider", &self.provider.name())
            .field("model", &self.model)
            .field("max_tool_iterations", &self.max_tool_iterations)
            .finish()
    }
}
impl AgentLoopImpl {
    pub fn new(config: AgentLoopConfig) -> Self {
        let context_manager = ContextManager::new(ContextManagerConfig {
            retention: config.retention.clone(),
            session_key: SessionIdentity::from_persisted_key(config.session_key.as_str()),
            max_context_tokens: config.max_context_tokens,
            pin_recent_turns: config.pin_recent_turns,
            context_marks: config.context_marks,
            model_context_window: config.model_context_window,
        });
        let mut agent = Self {
            unreported_usage: std::sync::Mutex::new(UsageTotals::default()),
            request_observations: std::sync::Mutex::new(Default::default()),
            in_flight_request: Arc::default(),
            request_tally: Arc::default(),
            interrupted_requests: std::sync::Mutex::new(Vec::new()),
            request_accounting: None,
            accounting_outbox: std::sync::Mutex::new(Vec::new()),
            tool_admission: None,
            request_prefix: std::sync::Mutex::new(None),
            input_baseline: Default::default(),
            request_admission: None,
            provider: config.provider,
            tool_registry: config.tool_registry,
            model: config.model.clone(),
            max_tokens: config.max_tokens,
            model_max_tokens: None,
            output_boost: std::sync::atomic::AtomicBool::new(false),
            last_request_max_tokens: std::sync::atomic::AtomicU32::new(0),
            temperature: config.temperature,
            max_tool_iterations: DEFAULT_MAX_TOOL_ITERATIONS,
            retains_context: config.retention.is_some(),
            session_key: config.session_key,
            model_context_window: config.model_context_window,
            model_traits: Default::default(),
            context_notes: Default::default(),
            progress_callback: config.progress_callback,
            streaming: config.streaming,
            effort: config.effort,
            default_effort: config.effort,
            audit_log: config.audit_log,
            durable_prefix_dirty: DurablePrefixLatch::shared(),
            context_manager,
            pending_tool_policy_requests: std::sync::Mutex::new(Vec::new()),
            tool_policy_state: std::sync::Mutex::new(ToolPolicyState::default()),
            turn_in_flight: std::sync::atomic::AtomicBool::new(false),
            tool_profile_context: config.tool_profile_context,
            tool_policy_persistence: None,
        };
        agent.sync_context_limits();
        agent
    }
    /// Read-and-clear the durable-prefix dirty latch (#1072): true when a
    /// pruning pass since the last take mutated existing history (stubs too).
    pub fn take_durable_prefix_dirty(&self) -> bool {
        self.durable_prefix_dirty.take()
    }
    /// Latch the durable-prefix dirty flag (called from the pruning pass).
    pub(super) fn latch_durable_prefix_dirty(&self) {
        self.durable_prefix_dirty.latch();
    }
    /// The latch the session save transaction drains (D5 #1972).
    pub fn durable_prefix_latch(&self) -> Arc<DurablePrefixLatch> {
        self.durable_prefix_dirty.clone()
    }
    /// Share a latch created elsewhere (a rig that swaps its agent).
    #[cfg(any(test, feature = "test-support"))]
    pub fn adopt_durable_prefix_latch(&mut self, latch: Arc<DurablePrefixLatch>) {
        self.durable_prefix_dirty = latch;
    }
    /// Replace the LLM provider after config reload. A reload may change the
    /// endpoint behind the same name (a rebuilt router is always `router`),
    /// so the calibration starts over, as on a model switch (#2212).
    pub fn swap_provider(&mut self, provider: Arc<dyn LlmProvider>) {
        self.provider = provider;
        self.context_manager.forget_calibration();
    }
    /// Return the currently configured model name.
    pub fn model(&self) -> &str {
        &self.model
    }
    /// Context-window ceiling (tokens), surfaced for UDS clients. Reports the
    /// window-aware effective budget — the same value pruning enforces
    /// (#1044) — so stats/snapshots never diverge from actual behaviour.
    pub fn max_context_tokens(&self) -> usize {
        self.effective_max_context_tokens()
    }
    /// Snapshot of the config-threaded context knobs
    /// `(pin_recent_turns, context_marks)` — observability
    /// for wiring checks so construction sites that drop user config are
    /// detectable from outside the loop (#1045/#1046). Test-gated: it exists
    /// only for wiring tests and must not ship as public API surface.
    #[cfg(test)]
    pub fn context_knob_snapshot(&self) -> (u32, Watermark) {
        self.context_manager.context_knob_snapshot()
    }
    /// Fire a progress event to the registered callback, if any. Takes a closure
    /// so the event is only constructed when a callback is registered; on the
    /// headless path (`progress_callback = None`) it's never called.
    #[inline]
    pub(super) fn notify(&self, make_event: impl FnOnce() -> AgentProgressEvent) {
        if let Some(ref cb) = self.progress_callback {
            cb(make_event());
        }
    }

    /// Replace the tool registry with a new one.
    pub fn swap_registry(&mut self, registry: Box<dyn ToolRegistry>) {
        self.tool_registry = registry;
    }
    /// Return names of tools registered from extensions (UDS `get_tool_catalogue`
    /// reports only actually-available tools; shadows are rejected earlier).
    pub fn runtime_tool_names(&self) -> Vec<String> {
        self.extension_tool_registry().runtime_tool_names()
    }
    /// Return descriptors for policy/UI callers without exposing concrete tool
    /// implementations.
    pub fn tool_descriptors(&self) -> Vec<ToolDescriptor> {
        self.tool_catalog().descriptors()
    }

    pub fn register_runtime_tool(
        &mut self,
        tool: std::sync::Arc<dyn crate::application::tools::ports::Tool>,
    ) -> bool {
        self.extension_tool_registry_mut()
            .register_runtime_tool(tool)
    }
    /// Register a single UDS-delivered extension tool.
    pub fn register_uds_tool(
        &mut self,
        tool: std::sync::Arc<dyn crate::application::tools::ports::Tool>,
    ) -> bool {
        self.extension_tool_registry_mut().register_uds_tool(tool)
    }
    /// Return whether a UDS-delivered extension tool would be accepted for a
    /// client owner without mutating the registry.
    pub fn can_register_uds_tool_for_owner(&self, name: &str, owner: &str) -> bool {
        self.extension_tool_registry()
            .can_register_uds_tool_for_owner(name, owner)
    }

    pub fn register_uds_tool_for_owner(
        &mut self,
        tool: std::sync::Arc<dyn crate::application::tools::ports::Tool>,
        owner: std::borrow::Cow<'static, str>,
    ) -> bool {
        self.extension_tool_registry_mut()
            .register_uds_tool_for_owner(tool, owner)
    }
    /// Unregister a single extension tool by name (e.g. on UDS client disconnect).
    pub fn unregister_runtime_tool(&mut self, name: &str) {
        self.unregister_runtime_tool_quiet(name);
    }

    /// Unregister one runtime tool without emitting a progress notification.
    ///
    /// UDS command dispatch batches catalogue change emission for a whole
    /// logical `unregister_tools` command so clients see one before/after event
    /// instead of one per tool plus one aggregate event.
    pub(crate) fn unregister_runtime_tool_quiet(&mut self, name: &str) {
        self.extension_tool_registry_mut()
            .unregister_runtime_tool(name);
    }

    /// Unregister all UDS-delivered extension tools owned by a connection.
    pub fn unregister_uds_tools_for_client(&mut self, client_id: u64) -> Vec<String> {
        let owner = format!("uds:client:{client_id}");
        let before = self.tool_catalogue_entries();
        let removed = self
            .tool_registry
            .unregister_runtime_tools_for_owner(owner.as_str());
        if !removed.is_empty() {
            self.notify_tool_catalogue_changed(removed.clone(), before, "unregister_client_tools");
        }
        removed
    }

    /// Return all tool definitions (for core name lookups).
    pub fn tool_definitions(&self) -> &[ToolDefinition] {
        self.tool_catalog().definitions()
    }

    pub(super) fn tool_catalog(&self) -> &dyn ToolCatalog {
        &*self.tool_registry
    }
    fn tool_executor(&self) -> &dyn ToolExecutor {
        &*self.tool_registry
    }
    pub(super) fn extension_tool_registry(&self) -> &dyn RuntimeToolLifecycleRegistry {
        &*self.tool_registry
    }
    fn extension_tool_registry_mut(&mut self) -> &mut dyn RuntimeToolLifecycleRegistry {
        &mut *self.tool_registry
    }

    fn session_aware_tools(&self) -> &dyn SessionAwareTools {
        &*self.tool_registry
    }
    /// Enable or disable incremental streaming for LLM calls.
    pub fn set_streaming(&mut self, enabled: bool) {
        self.streaming = enabled;
    }
    /// Set or replace the progress callback at runtime (UDS installs a
    /// streaming-token forwarder after construction; `None` clears it).
    pub fn set_progress_callback(&mut self, cb: Option<ProgressCallback>) {
        self.progress_callback = cb;
    }
    /// Access the audit log (if configured).
    pub fn audit_log(&self) -> Option<&Arc<dyn AuditSink>> {
        self.audit_log.as_ref()
    }

    /// Set or replace the audit log at runtime (wired by the UDS entry point
    /// after construction).
    pub fn set_audit_log(&mut self, log: Option<Arc<dyn AuditSink>>) {
        self.audit_log = log;
    }

    /// Write events to `log` from now on, when a session switch opened one
    /// for the arriving session (#2192 review); none keeps the current log.
    pub fn follow_audit_log(&mut self, log: Option<Arc<dyn AuditSink>>) {
        if let Some(log) = log {
            self.audit_log = Some(log);
        }
    }

    /// Emit an audit event if audit logging is enabled.
    ///
    /// Write failures are logged via `tracing::warn!` but never crash the agent.
    async fn audit(&self, turn: u32, event: AuditEvent) {
        if let Some(ref log) = self.audit_log {
            if let Err(e) = log.emit(turn, event).await {
                tracing::warn!(target: "audit", error = %e, "audit log write failed");
            }
        }
    }
}

#[cfg(test)]
#[path = "agent_loop_catalogue_tests.rs"]
mod catalogue_tests;
#[cfg(test)]
#[path = "agent_loop_cov_tests.rs"]
mod cov_tests;
#[cfg(test)]
#[path = "agent_loop_1072_tests.rs"]
mod issue_1072_tests;
#[cfg(test)]
#[path = "agent_loop_993_tests.rs"]
mod issue_993_tests;
#[cfg(test)]
#[path = "agent_loop_policy_tests.rs"]
mod policy_tests;
#[cfg(test)]
#[path = "agent_loop_spill_tests.rs"]
mod spill_tests;
#[cfg(test)]
#[path = "agent_loop_support_cov_tests.rs"]
mod support_cov_tests;
#[cfg(test)]
#[path = "agent_loop_swap_tests.rs"]
mod swap_tests;
#[cfg(test)]
#[path = "agent_loop_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "agent_loop_turn_tests.rs"]
mod turn_tests;

#[path = "agent_loop_accounting.rs"]
mod accounting;
