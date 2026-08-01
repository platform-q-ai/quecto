use crate::application::agent_loop_stream::{
    StreamProviderError, TurnEnd, empty_stream_error_message, is_empty_streamed_response,
};
use crate::application::agent_usage::UsageTotals;
use crate::application::context::{ContextManager, ContextManagerConfig};
use crate::application::context_pruning;
use crate::domain::agent::{
    AgentInfo, AgentLoop, AgentProgressEvent, AgentResult, ProgressCallback,
};
use crate::domain::audit::{AuditEvent, AuditSink};
use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, Message, ToolCall};
use crate::domain::provider::{ChatRequest, EffortLevel, LlmProvider, StreamEvent};
use crate::domain::provider_error::classify_provider_error;
use crate::domain::session::ContextSpillStore;
use crate::domain::tool::{
    RuntimeToolLifecycleRegistry, SessionAwareTools, ToolCatalog, ToolExecutor, ToolPolicyMutation,
    ToolRegistry,
};
use std::pin::Pin;
use std::sync::Arc;
#[path = "agent_loop_clamp.rs"]
mod agent_loop_clamp;
#[path = "agent_loop_effort.rs"]
mod agent_loop_effort;
#[path = "agent_loop_errors.rs"]
mod agent_loop_errors;
#[path = "agent_loop_preview.rs"]
pub(crate) mod agent_loop_preview;
#[path = "agent_loop_pruning.rs"]
mod agent_loop_pruning;
mod agent_loop_session;
#[path = "agent_loop_spill.rs"]
mod agent_loop_spill;
#[path = "agent_loop_tool_exec.rs"]
mod agent_loop_tool_exec;
#[path = "agent_loop_turn.rs"]
mod agent_loop_turn;
#[path = "agent_loop_turn_flow.rs"]
mod agent_loop_turn_flow;
use agent_loop_errors::{
    append_malformed_feedback, enhance_provider_error, is_context_or_output_limit_error,
    provider_failure_audit_event,
};
use agent_loop_spill::ToolMessageArgs;
use agent_loop_turn::{
    ProviderFailureTransition, TurnState, classify_provider_failure,
    next_state_after_provider_response, state_for_provider_failure_transition,
};
const DEFAULT_MAX_TOOL_ITERATIONS: u32 = 999_999;
const MAX_PROVIDER_ATTEMPTS: usize = 3;
const PROVIDER_RETRY_BACKOFF_MS: u64 = 100;
/// Cap on model-malformed requests re-prompted as addressable feedback (#931).
const MAX_MALFORMED_REQUEST_RETRIES: u32 = 3;
/// Configuration for building an agent loop.
pub struct AgentLoopConfig {
    pub provider: Arc<dyn LlmProvider>,
    pub tool_registry: Box<dyn ToolRegistry>,
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub spill_store: Option<Arc<dyn ContextSpillStore>>,
    pub session_key: String,
    pub context_collapse_after_tool_calls: u32,
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
    /// #1045: recent-turn tail-pin count for the pruning ceiling. A
    /// constructor field (not a post-construction builder) so no construction
    /// site can silently drop the user's configured value.
    pub pin_recent_turns: u32,
    /// #1046: count-based conversation-message collapse threshold
    /// (`u32::MAX` / `COLLAPSE_DISABLED` disables). Constructor field for the
    /// same reason as `pin_recent_turns`.
    pub context_collapse_after_messages: u32,
    /// #1044: the active model's known context window (`None` when unknown);
    /// bounds the effective pruning budget. Constructor field so window-aware
    /// budgeting cannot be forgotten at a construction site; `set_model`
    /// re-derives it on a model switch.
    pub model_context_window: Option<usize>,
}
pub struct AgentLoopImpl {
    provider: Arc<dyn LlmProvider>,
    pub(super) tool_registry: Box<dyn ToolRegistry>,
    model: String,
    max_tokens: u32,
    /// Per-model registry output cap, if known; see `agent_loop_clamp` (#935).
    model_max_tokens: Option<u32>,
    temperature: f32,
    max_tool_iterations: u32,
    spill_store: Option<Arc<dyn ContextSpillStore>>,
    session_key: String,
    /// #1044: the active model's known context window (None when unknown).
    pub(super) model_context_window: Option<usize>,
    /// When true, use incremental streaming for LLM calls.
    streaming: bool,
    /// Optional live progress callback wired by the REPL progress renderer.
    progress_callback: Option<ProgressCallback>,
    /// Optional effort level passed through to every ChatRequest.
    pub(super) effort: Option<EffortLevel>,
    /// Startup default effort, restored on session switches (#1067).
    pub(super) default_effort: Option<EffortLevel>,
    /// Optional append-only audit log for durable event recording.
    audit_log: Option<Arc<dyn AuditSink>>,
    /// #1072: latched by `apply_context_pruning` whenever a pass mutated
    /// existing history (in-place stub demotion, tool-result collapse, or a
    /// physical drop). Outcome-independent: it stays set across an Error or
    /// Cancelled turn so persistence can still reconcile. Consumed via
    /// [`Self::take_durable_prefix_dirty`].
    durable_prefix_dirty: std::sync::atomic::AtomicBool,
    /// Context-management boundary for pruning, spilling, dirty-prefix, and
    /// user-facing context gauge decisions.
    context_manager: ContextManager,
    pub(super) pending_tool_policy_mutations: std::sync::Mutex<Vec<ToolPolicyMutation>>,
    pub(super) runtime_disabled_tools: std::sync::Mutex<std::collections::HashSet<String>>,
    pub(super) runtime_enabled_tools: std::sync::Mutex<std::collections::HashSet<String>>,
    pub(super) turn_in_flight: std::sync::atomic::AtomicBool,
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
            spill_store: config.spill_store.clone(),
            session_key: config.session_key.clone(),
            context_collapse_after_tool_calls: config.context_collapse_after_tool_calls,
            max_context_tokens: config.max_context_tokens,
            pin_recent_turns: config.pin_recent_turns,
            context_collapse_after_messages: config.context_collapse_after_messages,
            model_context_window: config.model_context_window,
        });
        Self {
            provider: config.provider,
            tool_registry: config.tool_registry,
            model: config.model.clone(),
            max_tokens: config.max_tokens,
            model_max_tokens: None,
            temperature: config.temperature,
            max_tool_iterations: DEFAULT_MAX_TOOL_ITERATIONS,
            spill_store: config.spill_store,
            session_key: config.session_key,
            model_context_window: config.model_context_window,
            progress_callback: config.progress_callback,
            streaming: config.streaming,
            effort: config.effort,
            default_effort: config.effort,
            audit_log: config.audit_log,
            durable_prefix_dirty: std::sync::atomic::AtomicBool::new(false),
            context_manager,
            pending_tool_policy_mutations: std::sync::Mutex::new(Vec::new()),
            runtime_disabled_tools: std::sync::Mutex::new(std::collections::HashSet::new()),
            runtime_enabled_tools: std::sync::Mutex::new(std::collections::HashSet::new()),
            turn_in_flight: std::sync::atomic::AtomicBool::new(false),
        }
    }
    /// Read-and-clear the durable-prefix dirty latch (#1072).
    ///
    /// True when any pruning pass since the last take mutated already-existing
    /// history — including in-place stub demotion, which changes message
    /// CONTENT while every message id stays the same. Callers on the UDS
    /// dispatch path read this after EVERY prompt outcome (Success, Error,
    /// Cancelled) so persistence reconciles regardless of how the run ended.
    pub fn take_durable_prefix_dirty(&self) -> bool {
        self.durable_prefix_dirty
            .swap(false, std::sync::atomic::Ordering::Relaxed)
    }
    /// Latch the durable-prefix dirty flag (called from the pruning pass).
    pub(super) fn latch_durable_prefix_dirty(&self) {
        self.durable_prefix_dirty
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    /// Replace the LLM provider after config reload.
    pub fn swap_provider(&mut self, provider: Arc<dyn LlmProvider>) {
        self.provider = provider;
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
    /// `(pin_recent_turns, context_collapse_after_messages)` — observability
    /// for wiring checks so construction sites that drop user config are
    /// detectable from outside the loop (#1045/#1046). Test-gated: it exists
    /// only for wiring tests and must not ship as public API surface.
    #[cfg(test)]
    pub fn context_knob_snapshot(&self) -> (u32, u32) {
        self.context_manager.context_knob_snapshot()
    }
    fn reconcile_context_gauge(&self, estimate: usize) -> usize {
        self.context_manager.reconcile_context_gauge(estimate)
    }

    fn observe_provider_context_gauge(&self, reported_tokens: usize, estimate_at_call: usize) {
        self.context_manager
            .observe_provider_context_gauge(reported_tokens, estimate_at_call);
    }

    fn observe_estimated_context_gauge(&self, estimate: usize) {
        self.context_manager
            .observe_estimated_context_gauge(estimate);
    }

    #[doc(hidden)]
    pub fn reconcile_context_gauge_for_test(&self, estimate: usize) -> usize {
        self.reconcile_context_gauge(estimate)
    }

    #[doc(hidden)]
    pub fn observe_provider_context_gauge_for_test(
        &self,
        reported_tokens: usize,
        estimate_at_call: usize,
    ) {
        self.observe_provider_context_gauge(reported_tokens, estimate_at_call);
    }

    /// Poison the context-gauge mutex so coverage exercises the
    /// `unwrap_or_else(|e| e.into_inner())` recovery paths (#1128).
    #[cfg(test)]
    pub(super) fn poison_context_gauge_lock_for_test(&self) {
        self.context_manager.poison_context_gauge_lock_for_test();
    }

    /// Drive all three gauge entry points against a poisoned mutex.
    #[cfg(test)]
    pub(super) fn exercise_poisoned_context_gauge_for_test(&self) {
        self.poison_context_gauge_lock_for_test();
        assert_eq!(self.reconcile_context_gauge(42), 42);
        self.observe_provider_context_gauge(1_000, 100);
        assert_eq!(self.reconcile_context_gauge(80), 980);
        self.observe_estimated_context_gauge(1);
        assert_eq!(
            self.reconcile_context_gauge(80),
            980,
            "estimate-only must not clobber provider truth after poison recovery"
        );
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

    pub fn with_max_tool_iterations(mut self, max: u32) -> Self {
        self.max_tool_iterations = max;
        self
    }

    #[cfg(test)]
    pub fn with_progress_callback(mut self, callback: Option<ProgressCallback>) -> Self {
        self.progress_callback = callback;
        self
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

    /// Return rich additive catalogue/effective-policy state from the live
    /// registry for TUI/API callers.
    pub fn tool_catalogue_entries(
        &self,
    ) -> Vec<crate::domain::tool_descriptor::ToolCatalogueEntry> {
        self.tool_catalog().catalogue_entries()
    }

    pub fn register_runtime_tool(
        &mut self,
        tool: std::sync::Arc<dyn crate::domain::tool::Tool>,
    ) -> bool {
        self.extension_tool_registry_mut()
            .register_runtime_tool(tool)
    }

    /// Register a single UDS-delivered extension tool.
    pub fn register_uds_tool(
        &mut self,
        tool: std::sync::Arc<dyn crate::domain::tool::Tool>,
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
        tool: std::sync::Arc<dyn crate::domain::tool::Tool>,
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

    fn extension_tool_registry(&self) -> &dyn RuntimeToolLifecycleRegistry {
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

    /// Access the context spill store (if configured).
    pub fn spill_store(&self) -> Option<&Arc<dyn ContextSpillStore>> {
        self.spill_store.as_ref()
    }

    fn build_chat_request<'a>(
        &'a self,
        messages: &'a Vec<Message>,
        tool_defs: &'a [crate::domain::tool::ToolDefinition],
    ) -> ChatRequest<'a> {
        // Pass session_key as session_id so providers that support prompt
        // caching (e.g. Codex prompt_cache_key) can use it.
        let session_id = if self.session_key.is_empty() {
            None
        } else {
            Some(self.session_key.as_str())
        };
        ChatRequest {
            messages,
            tools: tool_defs,
            model: &self.model,
            max_tokens: self.effective_max_tokens(),
            temperature: self.temperature,
            session_id,
            tool_choice: None,
            metadata: None,
            thinking_level: None,
            cancel_flag: None,
            effort: self.effort,
        }
    }

    async fn finalize_text_response(
        &self,
        messages: &mut Vec<Message>,
        response: LlmResponse,
        end: TurnEnd,
    ) -> AgentResult {
        let text = response.content.unwrap_or_default();
        let mut assistant_message = Message::assistant(text.clone(), vec![]);
        // Stamp + spill at creation: the loop returns right after this, so no
        // later pruning pass could file the final reply (#1046).
        assistant_message.turn = Some(end.current_turn);
        self.spill_conversation_message(&mut assistant_message)
            .await;
        let estimate_context_tokens = end
            .pre_response_context_tokens
            .saturating_add(context_pruning::estimate_message_tokens(&assistant_message));
        messages.push(assistant_message);
        let usage = end.usage;
        let context_tokens = if usage.context_input_tokens > 0 {
            usage.context_input_tokens as usize
        } else {
            estimate_context_tokens
        };
        if usage.context_input_tokens > 0 {
            self.observe_provider_context_gauge(context_tokens, end.pre_response_context_tokens);
        } else {
            self.observe_estimated_context_gauge(estimate_context_tokens);
        }
        AgentResult {
            response: text,
            tool_iterations: end.iterations,
            iteration_limit_reached: false,
            input_tokens: usage.context_input_tokens,
            context_tokens,
            output_tokens: usage.output_tokens,
            billed_input_tokens: usage.billed_input_tokens,
            billed_output_tokens: usage.billed_output_tokens,
            cache_read_tokens: usage.cache_read_tokens,
            cache_write_tokens: usage.cache_write_tokens,
            cost_micro_usd: usage.cost_micro_usd,
            appended_messages: Vec::new(),
        }
    }

    /// Send a chat request using incremental streaming.
    ///
    /// Emits `AgentProgressEvent::Token` for each text delta so the UDS layer
    /// can forward them as `{"type":"token"}` events.  Falls back gracefully
    /// for providers whose `chat_stream_incremental()` wraps `chat()` (emitting
    /// only a single `Done`).
    async fn stream_chat_once(
        &self,
        request: ChatRequest<'_>,
    ) -> Result<LlmResponse, StreamProviderError> {
        let mut emitted_event = false;
        let mut rx = self.provider.chat_stream_incremental(request).await;
        while let Some(event) = rx.recv().await {
            match event {
                StreamEvent::TextDelta(t) => {
                    emitted_event = true;
                    self.notify(|| AgentProgressEvent::Token(t));
                }
                StreamEvent::Done(response) => {
                    if is_empty_streamed_response(&response) {
                        return Err(StreamProviderError {
                            error: DomainError::Provider(empty_stream_error_message(&response)),
                            emitted_event,
                        });
                    }
                    return Ok(response);
                }
                StreamEvent::Error(e) => {
                    return Err(StreamProviderError {
                        error: DomainError::Provider(e),
                        emitted_event,
                    });
                }
                // Tool call streaming events are handled by the provider's
                // accumulator — they assemble into LlmResponse.tool_calls
                // and are delivered via StreamEvent::Done.
                _ => {
                    emitted_event = true;
                }
            }
        }
        // Channel closed without Done — shouldn't happen but handle gracefully.
        Err(StreamProviderError {
            error: DomainError::Provider("streaming channel closed without completion".to_string()),
            emitted_event,
        })
    }

    /// Run the LLM-tool loop.
    async fn run_loop(&self, messages: &mut Vec<Message>) -> Result<AgentResult, DomainError> {
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

            let request = self.prepare_provider_request_transition(
                messages,
                &tool_defs,
                estimated_context_tokens,
            );
            let _state = TurnState::AwaitProviderResponse;
            self.audit_provider_request_start(
                current_turn,
                estimated_context_tokens,
                messages.len(),
            )
            .await;

            let llm_start = std::time::Instant::now();
            // Streaming (UDS mode) forwards token events in real time; REPL/
            // one-shot use the non-streaming path.
            let response = self.request_provider_response(request).await;

            let llm_duration_ms = llm_start.elapsed().as_millis() as u64;
            let response = match response {
                Ok(response) => response,
                Err(error) => {
                    let transition = classify_provider_failure(
                        &error,
                        malformed_retries,
                        MAX_MALFORMED_REQUEST_RETRIES,
                    );
                    let _state = state_for_provider_failure_transition(&transition);
                    match transition {
                        ProviderFailureTransition::RecoverMalformedRequest => {
                            let _state = TurnState::RecoverMalformedResponse;
                            self.recover_malformed_response(
                                messages,
                                &error,
                                current_turn,
                                &mut malformed_retries,
                                &mut appended_messages,
                            )
                            .await;
                            current_turn += 1;
                            continue;
                        }
                        ProviderFailureTransition::Terminal(_class) => {
                            let _state = TurnState::FailProviderRequest;
                            self.clear_turn_in_flight();
                            return self.fail_provider_request(current_turn, error).await;
                        }
                    }
                }
            };

            self.audit_provider_response_end(
                current_turn,
                &response,
                estimated_context_tokens,
                llm_duration_ms,
            )
            .await;

            if let Some(ref usage) = response.usage {
                usage_totals.record(usage);
            }

            match next_state_after_provider_response(&response) {
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
                    self.clear_turn_in_flight();
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
            spills_dirty = self.spill_store.is_some();
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
                self.clear_turn_in_flight();
                return Ok(result);
            }
        }
    }
}

impl AgentLoop for AgentLoopImpl {
    fn process<'a>(
        &'a self,
        messages: &'a mut Vec<Message>,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<AgentResult, DomainError>> + Send + 'a>>
    {
        Box::pin(self.run_loop(messages))
    }

    fn info(&self) -> AgentInfo {
        AgentInfo {
            tool_count: self.tool_catalog().tool_count(),
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
