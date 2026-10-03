use super::durable_prefix::DurablePrefixLatch;
use crate::application::agent_loop_policy::ToolPolicyState;
use crate::application::agent_loop_stream::{
    StreamProviderError, TurnEnd, empty_stream_error_message, is_cut_off_without_answer,
    is_empty_streamed_response,
};
use crate::application::agent_turn::ports::AgentLoop;
pub use crate::application::agent_usage::UsageTotals;
use crate::application::audit::ports::AuditSink;
use crate::application::context::{ContextManager, ContextManagerConfig};
use crate::application::context_pruning;
use crate::application::providers::ports::LlmProvider;
use crate::application::tools::ports::{
    RuntimeToolLifecycleRegistry, SessionAwareTools, ToolCatalog, ToolExecutor, ToolRegistry,
};
use crate::domain::agent::{AgentInfo, AgentProgressEvent, AgentResult, ProgressCallback};
use crate::domain::audit::AuditEvent;
use crate::domain::conversation::reply_requirement::ReplyRequirement;
use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, Message, ToolCall};
use crate::domain::provider::{EffortLevel, StreamEvent};
use crate::domain::provider_error::classify_provider_error;
use crate::domain::tool::ToolProfileContext;
use std::pin::Pin;
use std::sync::Arc;

pub type ToolPolicyPersistence =
    Arc<dyn Fn(&crate::domain::tool::ToolPolicyReconciliation) -> Result<(), String> + Send + Sync>;
#[path = "agent_loop_clamp.rs"]
mod agent_loop_clamp;
#[path = "agent_loop_effort.rs"]
mod agent_loop_effort;
#[path = "agent_loop_errors.rs"]
mod agent_loop_errors;
#[path = "agent_loop_gauge.rs"]
mod agent_loop_gauge;
#[path = "agent_loop_preview.rs"]
pub(crate) mod agent_loop_preview;
#[path = "agent_loop_pruning.rs"]
mod agent_loop_pruning;
#[path = "agent_loop_reload.rs"]
mod agent_loop_reload;
mod agent_loop_session;
#[path = "agent_loop_spill.rs"]
mod agent_loop_spill;
#[path = "agent_loop_tool_exec.rs"]
mod agent_loop_tool_exec;
#[path = "agent_loop_turn.rs"]
mod agent_loop_turn;
#[path = "agent_loop_turn_flow.rs"]
mod agent_loop_turn_flow;
#[path = "agent_loop_uds_tools.rs"]
mod agent_loop_uds_tools;
use agent_loop_errors::{
    append_feedback, append_malformed_feedback, enhance_provider_error,
    is_context_or_output_limit_error, output_limit_feedback, provider_failure_audit_event,
    record_feedback,
};
use agent_loop_spill::ToolMessageArgs;
use agent_loop_turn::{
    Output, ProviderFailureTransition, TurnState, classify_provider_failure,
    next_state_after_provider_response, state_for_provider_failure_transition,
};
use agent_loop_turn_flow::AfterResponse;
const DEFAULT_MAX_TOOL_ITERATIONS: u32 = 999_999;
const MAX_PROVIDER_ATTEMPTS: usize = 3;
const PROVIDER_RETRY_BACKOFF_MS: u64 = 100;
/// Cap on model-malformed requests re-prompted as addressable feedback (#931).
const MAX_MALFORMED_REQUEST_RETRIES: u32 = 3;
pub struct AgentLoopConfig {
    pub provider: Arc<dyn LlmProvider>,
    pub tool_registry: Box<dyn ToolRegistry>,
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
    /// The loop's retention handles (D9 #1978), composed over the one
    /// retention store; `None` retains nothing.
    pub retention: Option<crate::application::context::ContextRetention>,
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
    pub context_marks: crate::domain::conversation::watermark::Watermark,
    /// #1044: active model context window (`None` unknown); bounds pruning budget.
    pub model_context_window: Option<usize>,
    pub tool_profile_context: ToolProfileContext,
}
pub struct AgentLoopImpl {
    unreported_usage: std::sync::Mutex<UsageTotals>,
    accounting_outbox:
        std::sync::Mutex<Vec<crate::domain::request_observation::RequestObservation>>,
    request_observations: std::sync::Mutex<crate::domain::request_observation::RequestDiagnostics>,
    /// The request in flight, which `get_state` reports (#2210).
    in_flight_request: Arc<crate::domain::request_progress::InFlightRequest>,
    /// Requests that ended in flight, awaiting their audit record (#2210).
    interrupted_requests: std::sync::Mutex<Vec<super::request_observation::InterruptedRequest>>,
    request_admission: Option<Arc<dyn crate::application::providers::ports::RequestAdmission>>,
    request_accounting: Option<Arc<dyn crate::application::providers::ports::RequestAccounting>>,
    tool_admission: Option<Arc<dyn crate::application::tools::ports::ToolExecutionAdmission>>,
    request_prefix: std::sync::Mutex<Option<String>>,
    /// The session's input baseline, which outlives a rebuilt provider (#2398).
    input_baseline: crate::domain::request_observation::InputBaseline,
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
    pub(super) pending_tool_policy_requests:
        std::sync::Mutex<Vec<crate::domain::tool::ToolPolicyRequest>>,
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
            session_key: crate::domain::session_identity::SessionIdentity::from_persisted_key(
                config.session_key.as_str(),
            ),
            max_context_tokens: config.max_context_tokens,
            pin_recent_turns: config.pin_recent_turns,
            context_marks: config.context_marks,
            model_context_window: config.model_context_window,
        });
        let mut agent = Self {
            unreported_usage: std::sync::Mutex::new(UsageTotals::default()),
            request_observations: std::sync::Mutex::new(Default::default()),
            in_flight_request: Arc::default(),
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
    pub fn context_knob_snapshot(
        &self,
    ) -> (u32, crate::domain::conversation::watermark::Watermark) {
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
    pub fn tool_descriptors(&self) -> Vec<crate::domain::tool_descriptor::ToolDescriptor> {
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
    pub fn tool_definitions(&self) -> &[crate::domain::tool::ToolDefinition] {
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

    /// Run the LLM-tool loop.
    async fn run_loop(&mut self, messages: &mut Vec<Message>) -> Result<AgentResult, DomainError> {
        self.mark_turn_in_flight();
        let mut tool_defs = self.current_tool_definitions();
        let mut iterations: u32 = 0;
        let mut current_turn: u32 = 1;
        // True when the manifest needs a rebuild; starts true for prior spills.
        let mut spills_dirty = true;
        let mut usage_totals = UsageTotals::default();
        // Per-run append ledger (#1072). Entries are full clones taken at
        // append time: a later ladder pass may demote any of them IN PLACE
        // (`collapse_message` mutates the `&mut Message` in the conversation
        // vector), so a shared representation (`Arc<Message>`) or clone-on-emit
        // cannot preserve as-appended content.
        //
        // Cost (#1073 review): one clone per appended message, held until run
        // end. Accepted deliberately: the ledger drops when `AgentResult`
        // is consumed at run end, and emission is independently capped at the
        // shared 8 MiB protocol frame cap (#1062); over-cap aggregates reject
        // at emission. If run-lifetime ledger growth becomes a problem, use
        // clone-on-demote (move the original into the ledger only when a
        // prune pass is about to mutate it), not Arc sharing.
        let mut appended_messages = Vec::new();
        // #1072: durable-prefix dirtiness is latched by `apply_context_pruning`
        // itself (any mutating ladder/collapse outcome) rather than diffing a
        // pre-run id snapshot — in-place stub demotion changes content while
        // every message id stays the same, which a snapshot comparison misses.
        // Count of model-malformed requests turned into addressable feedback.
        let mut malformed_retries: u32 = 0;
        let mut cut_off_retries: u32 = 0;

        loop {
            if iterations > 0 {
                self.drain_tool_policy_mutations_at_internal_boundary();
                tool_defs = self.current_tool_definitions();
            }
            let _state = TurnState::PrepareProviderRequest;
            let estimated_context_tokens = self
                .apply_context_pruning(messages, current_turn, spills_dirty)
                .await;
            self.notify(|| AgentProgressEvent::ConversationChanged {
                messages: messages.clone().into(),
            });

            // #2421: the conversation as the active model is sent it.
            let sent = self.conversation_for_model(messages);
            // #2434: whether its reply may be empty (tool results) or must
            // have output (a prompt, a steer, a follow-up, feedback).
            let requirement = ReplyRequirement::for_conversation(sent.messages());
            let request = self.prepare_provider_request_transition(
                sent.messages(),
                &tool_defs,
                estimated_context_tokens,
            );
            let _state = TurnState::AwaitProviderResponse;
            let message_count = sent.messages().len();
            self.audit_provider_request_start(
                current_turn,
                estimated_context_tokens,
                message_count,
            )
            .await;

            let llm_start = std::time::Instant::now();
            // Streaming (UDS mode) forwards token events in real time; REPL/
            // one-shot use the non-streaming path.
            let (response, output) = match self
                .request_provider_response(
                    request,
                    (current_turn, estimated_context_tokens),
                    requirement,
                )
                .await
            {
                Ok(response) => (Ok(response), Output::NotShown),
                Err(failure) => (
                    Err(failure.error),
                    Output::from_emitted(failure.emitted_event),
                ),
            };
            drop(sent);

            let llm_duration_ms = llm_start.elapsed().as_millis() as u64;
            // Every response is audited and counted, including one dropped below.
            let timing = (current_turn, estimated_context_tokens, llm_duration_ms);
            self.account_response(&response, timing, &mut usage_totals)
                .await;
            let after = match response {
                // #2124: nothing visible before the output limit is no answer.
                Ok(response)
                    if is_cut_off_without_answer(&response, self.last_request_max_tokens()) =>
                {
                    let feedback = output_limit_feedback(self.last_request_max_tokens());
                    let recovery = (&mut cut_off_retries, &mut appended_messages, feedback);
                    self.after_cut_off_answer(messages, &response, current_turn, recovery)
                        .await
                }
                Ok(response) => AfterResponse::Proceed(response),
                Err(error) => {
                    let recovery = (&mut malformed_retries, &mut appended_messages);
                    self.after_provider_failure(messages, (error, output), current_turn, recovery)
                        .await
                }
            };
            let response = match after {
                AfterResponse::Proceed(response) => {
                    cut_off_retries = 0;
                    response
                }
                AfterResponse::Retry => {
                    current_turn += 1;
                    continue;
                }
                AfterResponse::Finish(result) => return result,
            };

            match next_state_after_provider_response(&response, requirement) {
                TurnState::FinalizeAssistantResponse => {
                    let end = TurnEnd {
                        iterations,
                        usage: usage_totals,
                        pre_response_context_tokens: estimated_context_tokens,
                        current_turn,
                    };
                    let result = self
                        .finalize_turn_response(messages, response, end, &mut appended_messages)
                        .await;
                    self.drain_tool_policy_mutations_at_boundary();
                    return Ok(result);
                }
                TurnState::EndTurnWithoutReply => {
                    let result = self.empty_reply_result(
                        messages,
                        iterations,
                        usage_totals,
                        appended_messages,
                    );
                    self.drain_tool_policy_mutations_at_boundary();
                    return Ok(result);
                }
                TurnState::ExecuteToolCalls => {}
                _ => unreachable!("provider response classification returned non-response state"),
            }

            // #1072: this turn's appended messages are recorded in the run
            // ledger AT APPEND TIME inside `execute_tool_calls_for_response`
            // — never recovered from a positional slice of `messages`, which
            // pruning can shrink or demote in place.
            let ledger_from = appended_messages.len();
            self.execute_tool_calls_for_response(
                messages,
                current_turn,
                response,
                &mut appended_messages,
            )
            .await;
            // Stream this turn's output (assistant message + tool results) over
            // the live progress path so a parent/inspector sees it turn-by-turn,
            // not only at completion (#797). The clone is only paid when a
            // progress callback is registered (via `notify`'s guard), and the
            // Arc<[Message]> payload makes further event clones refcount bumps (#993).
            self.notify(|| AgentProgressEvent::TurnCompleted {
                messages: appended_messages[ledger_from..].into(),
            });
            // Tool calls were executed and spilled — mark dirty for next iteration
            spills_dirty = self.retains_context;
            iterations += 1;
            current_turn += 1;

            if iterations >= self.max_tool_iterations {
                let _state = TurnState::StopAtToolIterationLimit;
                let result = self.tool_iteration_limit_result(
                    messages,
                    iterations,
                    usage_totals,
                    appended_messages,
                );
                self.drain_tool_policy_mutations_at_boundary();
                return Ok(result);
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
