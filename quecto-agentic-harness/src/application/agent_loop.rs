use std::pin::Pin;
use std::sync::Arc;

use crate::application::agent_usage::UsageTotals;
use crate::application::context_pruning;
use crate::domain::agent::{
    AgentInfo, AgentLoop, AgentProgressEvent, AgentResult, ProgressCallback,
};
use crate::domain::audit::{AuditEvent, AuditSink};
use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, Message, ToolCall};
use crate::domain::provider::{ChatRequest, EffortLevel, LlmProvider, StreamEvent};
use crate::domain::provider_error::{ProviderErrorClass, classify_provider_error};
use crate::domain::session::ContextSpillStore;
use crate::domain::tool::ToolRegistry;

#[path = "agent_loop_clamp.rs"]
mod agent_loop_clamp;
#[path = "agent_loop_errors.rs"]
mod agent_loop_errors;
#[path = "agent_loop_preview.rs"]
pub(crate) mod agent_loop_preview;
#[path = "agent_loop_pruning.rs"]
mod agent_loop_pruning;
mod agent_loop_session;
#[path = "agent_loop_spill.rs"]
mod agent_loop_spill;
use agent_loop_errors::{
    append_malformed_feedback, enhance_provider_error, is_context_or_output_limit_error,
    provider_failure_audit_event,
};
use agent_loop_spill::ToolMessageArgs;

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
    /// Optional live progress events callback (REPL renderer); `None` for
    /// headless operation (no-op, zero overhead).
    pub progress_callback: Option<ProgressCallback>,
    /// When `true`, use `chat_stream_incremental()` so Token events stream in
    /// real time (UDS path); `false` for REPL (non-streaming mocks in tests).
    pub streaming: bool,
    /// Optional effort level for 4.6 models. When `Some`, passed through to
    /// every `ChatRequest`. When `None`, the provider applies its own default.
    pub effort: Option<EffortLevel>,
    /// Optional dynamic system prompt provider invoked before each LLM turn.
    pub system_prompt_provider: Option<Arc<dyn Fn() -> String + Send + Sync>>,
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
    tool_registry: Box<dyn ToolRegistry>,
    model: String,
    max_tokens: u32,
    /// Per-model registry output cap, if known; see `agent_loop_clamp` (#935).
    model_max_tokens: Option<u32>,
    temperature: f32,
    max_tool_iterations: u32,
    spill_store: Option<Arc<dyn ContextSpillStore>>,
    session_key: String,
    context_collapse_after_tool_calls: u32,
    max_context_tokens: usize,
    /// #1045: recent-turn tail-pin count for the spilling ceiling.
    pub(super) pin_recent_turns: u32,
    /// #1046: count-based conversation-message collapse threshold.
    pub(super) context_collapse_after_messages: u32,
    /// #1044: the active model's known context window (None when unknown).
    pub(super) model_context_window: Option<usize>,
    /// When true, use incremental streaming for LLM calls.
    streaming: bool,
    /// Optional live progress callback wired by the REPL progress renderer.
    progress_callback: Option<ProgressCallback>,
    /// Optional effort level passed through to every ChatRequest.
    effort: Option<EffortLevel>,
    /// Optional dynamic system prompt provider invoked before each LLM turn.
    system_prompt_provider: Option<Arc<dyn Fn() -> String + Send + Sync>>,
    /// Optional append-only audit log for durable event recording.
    audit_log: Option<Arc<dyn AuditSink>>,
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

struct StreamProviderError {
    error: DomainError,
    emitted_event: bool,
}

/// End-of-loop bookkeeping for the final text response (avoids the clippy
/// argument-count limit on `finalize_text_response`).
struct TurnEnd {
    iterations: u32,
    usage: UsageTotals,
    pre_response_context_tokens: usize,
    current_turn: u32,
}

impl AgentLoopImpl {
    pub fn new(config: AgentLoopConfig) -> Self {
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
            context_collapse_after_tool_calls: config.context_collapse_after_tool_calls,
            max_context_tokens: config.max_context_tokens,
            pin_recent_turns: config.pin_recent_turns,
            context_collapse_after_messages: config.context_collapse_after_messages,
            model_context_window: config.model_context_window,
            progress_callback: config.progress_callback,
            streaming: config.streaming,
            effort: config.effort,
            system_prompt_provider: config.system_prompt_provider,
            audit_log: config.audit_log,
        }
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
        (self.pin_recent_turns, self.context_collapse_after_messages)
    }

    /// Fire a progress event to the registered callback, if any. Takes a closure
    /// so the event is only constructed when a callback is registered; on the
    /// headless path (`progress_callback = None`) it's never called.
    #[inline]
    fn notify(&self, make_event: impl FnOnce() -> AgentProgressEvent) {
        if let Some(ref cb) = self.progress_callback {
            cb(make_event());
        }
    }

    /// Set the maximum number of tool iterations (overrides default).
    pub fn with_max_tool_iterations(mut self, max: u32) -> Self {
        self.max_tool_iterations = max;
        self
    }

    /// Replace the tool registry with a new one.
    pub fn swap_registry(&mut self, registry: Box<dyn ToolRegistry>) {
        self.tool_registry = registry;
    }

    /// Return names of tools registered from extensions (UDS `get_extensions`
    /// reports only actually-available tools; shadows are rejected earlier).
    pub fn tool_registry_extension_names(&self) -> Vec<String> {
        self.tool_registry.extension_names()
    }

    /// Register a single extension tool (e.g. from a UDS client).
    pub fn register_extension_tool(&mut self, tool: std::sync::Arc<dyn crate::domain::tool::Tool>) {
        crate::domain::tool::ToolRegistry::register_extension(&mut *self.tool_registry, tool);
    }

    /// Unregister a single extension tool by name (e.g. on UDS client disconnect).
    pub fn unregister_extension_tool(&mut self, name: &str) {
        crate::domain::tool::ToolRegistry::unregister_extension(&mut *self.tool_registry, name);
    }

    /// Return all tool definitions (for core name lookups).
    pub fn tool_definitions(&self) -> &[crate::domain::tool::ToolDefinition] {
        self.tool_registry.definitions()
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

    /// Set or replace the dynamic system prompt provider at runtime.
    pub fn set_system_prompt_provider(
        &mut self,
        provider: Option<Arc<dyn Fn() -> String + Send + Sync>>,
    ) {
        self.system_prompt_provider = provider;
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

    fn refresh_dynamic_system_prompt(&self, messages: &mut Vec<Message>) {
        let Some(ref provider) = self.system_prompt_provider else {
            return;
        };
        let prompt = provider();
        if prompt.is_empty() {
            return;
        }
        if let Some(first) = messages.first_mut()
            && first.role == crate::domain::message::Role::System
            && !first.is_manifest
        {
            if first.content != prompt {
                first.content = prompt;
                first.invalidate_token_cache();
            }
        } else {
            messages.insert(0, Message::system(prompt));
        }
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

    async fn execute_tool_calls_for_response(
        &self,
        messages: &mut Vec<Message>,
        current_turn: u32,
        response: LlmResponse,
    ) {
        let mut assistant =
            Message::assistant(response.content.unwrap_or_default(), response.tool_calls);
        assistant.stop_reason = response.stop_reason;
        assistant.thinking_blocks = response.thinking_blocks;
        // Stamp the turn: the creation-time spill files this as
        // turn{N}:msg:assistant (#1046).
        assistant.turn = Some(current_turn);
        self.spill_conversation_message(&mut assistant).await;
        messages.push(assistant);
        let assistant_index = messages.len() - 1;
        let tool_call_count = messages[assistant_index].tool_calls.len();

        for idx in 0..tool_call_count {
            let tc = &messages[assistant_index].tool_calls[idx];
            // Audit: ToolCall (guarded — avoid clones when audit is disabled)
            if self.audit_log.is_some() {
                self.audit(
                    current_turn,
                    AuditEvent::ToolCall {
                        tool: tc.name.clone(),
                        call_id: tc.id.clone(),
                        arguments: tc.arguments.clone(),
                    },
                )
                .await;
            }

            let (content, image_blocks, is_error) = self.execute_single_tool_call(tc).await;

            // Audit: ToolResult (guarded — avoid estimate_tokens/preview when disabled)
            if self.audit_log.is_some() {
                let content_tokens = context_pruning::estimate_tokens(&content);
                let preview = crate::domain::audit::content_preview(&content, 200);
                self.audit(
                    current_turn,
                    AuditEvent::ToolResult {
                        call_id: tc.id.clone(),
                        tool: tc.name.clone(),
                        is_error,
                        content_tokens,
                        content_preview: preview,
                    },
                )
                .await;
            }

            let spill_id = format!("turn{}:{}:{}", current_turn, tc.name, idx);
            let mut tool_msg = self.build_tool_message(ToolMessageArgs {
                tc,
                content,
                image_blocks,
                is_error,
            });
            tool_msg.turn = Some(current_turn);
            // Stamps `spill_id` on the message only if the append succeeds.
            self.spill_tool_message(&mut tool_msg, spill_id).await;
            messages.push(tool_msg);
        }
    }

    async fn execute_single_tool_call(
        &self,
        tc: &ToolCall,
    ) -> (String, Vec<crate::domain::tool::ImageBlock>, bool) {
        // Emit ToolStarted before executing so the REPL can show the tool name
        // immediately, even if the tool itself takes a long time.
        // Clones inside the closure are only evaluated when a callback is
        // registered (zero-cost on headless paths via notify's guard).
        self.notify(|| AgentProgressEvent::ToolStarted {
            tool_call_id: tc.id.clone(),
            name: tc.name.clone(),
            arguments: tc.arguments.clone(),
        });

        let start = std::time::Instant::now();
        let tool_result = self.tool_registry.execute(&tc.name, &tc.arguments).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        let is_err = tool_result.is_err();
        let (content, image_blocks) = match tool_result {
            Ok(tr) => (tr.content, tr.image_blocks),
            Err(e) => (format!("Error: {}", e), vec![]),
        };

        // Emit ToolFinished so the REPL can replace the spinner line.
        // Build the bounded preview inside notify so headless runs allocate none.
        self.notify(|| AgentProgressEvent::ToolFinished {
            tool_call_id: tc.id.clone(),
            name: tc.name.clone(),
            arguments: tc.arguments.clone(),
            result_content: agent_loop_preview::tool_result_preview(&content),
            duration_ms,
            is_error: is_err,
        });

        tracing::info!(
            target: "tool_exec",
            tool_name = tc.name.as_str(),
            duration_ms,
            is_error = is_err,
            "tool executed"
        );
        (content, image_blocks, is_err)
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
        let context_tokens = end
            .pre_response_context_tokens
            .saturating_add(context_pruning::estimate_message_tokens(&assistant_message));
        messages.push(assistant_message);
        let usage = end.usage;
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
                StreamEvent::Done(response) => return Ok(response),
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

    async fn call_provider_with_retries(
        &self,
        request: ChatRequest<'_>,
    ) -> Result<LlmResponse, DomainError> {
        // Transient-error retry is owned by the `RetryingProvider` decorator, so
        // the non-streaming path makes a single call and passes the error
        // through (only enhancing it); re-retrying here would double the budget.
        if !self.streaming {
            return self
                .provider
                .chat(request)
                .await
                .map_err(enhance_provider_error);
        }

        // Streaming initiation *is* retried here: the decorator forwards
        // `chat_stream` without retry, so this loop owns stream re-initiation.
        for attempt in 1..=MAX_PROVIDER_ATTEMPTS {
            let result = match self.stream_chat_once(request.clone()).await {
                Ok(response) => Ok(response),
                Err(stream_error) if stream_error.emitted_event => {
                    // Replaying after emitted content would corrupt output.
                    return Err(enhance_provider_error(stream_error.error));
                }
                Err(stream_error) => Err(stream_error.error),
            };

            match result {
                Ok(response) => return Ok(response),
                Err(err) => {
                    let class = classify_provider_error(&err);
                    if attempt == MAX_PROVIDER_ATTEMPTS || !class.is_retryable() {
                        return Err(enhance_provider_error(err));
                    }
                    tracing::warn!(
                        target: "provider_retry",
                        attempt,
                        max_attempts = MAX_PROVIDER_ATTEMPTS,
                        error_class = %class,
                        "retrying stream initiation after transient failure"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(
                        PROVIDER_RETRY_BACKOFF_MS * attempt as u64,
                    ))
                    .await;
                }
            }
        }

        Err(DomainError::Provider(
            "provider request failed without an error".to_string(),
        ))
    }

    /// Run the LLM-tool loop.
    async fn run_loop(&self, messages: &mut Vec<Message>) -> Result<AgentResult, DomainError> {
        let tool_defs = self.tool_registry.definitions();
        let mut iterations: u32 = 0;
        let mut current_turn: u32 = 1;
        // True when the manifest needs a rebuild; starts true for prior spills.
        let mut spills_dirty = true;
        let mut usage_totals = UsageTotals::default();
        // Count of model-malformed requests turned into addressable feedback.
        let mut malformed_retries: u32 = 0;

        loop {
            self.refresh_dynamic_system_prompt(messages);
            let context_tokens = self
                .apply_context_pruning(messages, current_turn, spills_dirty)
                .await;

            // Emit Thinking before every LLM call so the REPL spinner activates
            // immediately, including during multi-turn tool loops.
            self.notify(|| AgentProgressEvent::Thinking {
                context_tokens,
                max_context_tokens: self.effective_max_context_tokens(),
                provider: self.provider.name().to_string(),
                model: self.model.clone(),
            });

            let request = self.build_chat_request(messages, tool_defs);
            // Audit: LlmTurnStart (guarded)
            if self.audit_log.is_some() {
                self.audit(
                    current_turn,
                    AuditEvent::LlmTurnStart {
                        input_tokens_estimate: context_tokens,
                        message_count: messages.len(),
                    },
                )
                .await;
            }

            let llm_start = std::time::Instant::now();
            // Streaming (UDS mode) forwards token events in real time; REPL/
            // one-shot use the non-streaming path.
            let response = self.call_provider_with_retries(request).await;

            let llm_duration_ms = llm_start.elapsed().as_millis() as u64;
            let response = match response {
                Ok(r) => r,
                Err(e) => {
                    // A model-malformed `Client`/4xx rejection must not kill the
                    // turn: re-prompt with addressable feedback so the model
                    // self-corrects (#931 AC2), bounded by MAX_MALFORMED_REQUEST_
                    // RETRIES. Context/output-limit 4xx is terminal (below).
                    let is_malformed_request = matches!(&e, DomainError::Provider(m)
                        if classify_provider_error(&e) == ProviderErrorClass::Client
                            && !is_context_or_output_limit_error(m));
                    if is_malformed_request && malformed_retries < MAX_MALFORMED_REQUEST_RETRIES {
                        malformed_retries += 1;
                        tracing::warn!(
                            target: "provider_retry",
                            attempt = malformed_retries,
                            max = MAX_MALFORMED_REQUEST_RETRIES,
                            error = %e,
                            "provider rejected request as malformed — re-prompting with addressable feedback"
                        );
                        append_malformed_feedback(messages, &e, current_turn);
                        current_turn += 1;
                        continue;
                    }
                    // Audit: persist the FULL provider error body (redacted)
                    // once per terminal failure, never per retry, so it survives
                    // TUI line-truncation (#937). `provider` is the harness
                    // adapter name (e.g. `openai`), not the upstream endpoint
                    // (#939 review).
                    let ev = provider_failure_audit_event(self.provider.name(), &e);
                    self.audit(current_turn, ev).await;
                    self.notify(|| AgentProgressEvent::Done);
                    return Err(e);
                }
            };

            // Audit: LlmTurnEnd (guarded)
            if self.audit_log.is_some() {
                let (input_toks, output_toks) = response
                    .usage
                    .as_ref()
                    .map(|u| (u.context_input_tokens() as _, u.completion_tokens as _))
                    .unwrap_or((context_tokens, 0));
                let stop = response
                    .stop_reason
                    .as_ref()
                    .map_or_else(|| "unknown".to_string(), |s| s.to_string());
                self.audit(
                    current_turn,
                    AuditEvent::LlmTurnEnd {
                        input_tokens: input_toks,
                        output_tokens: output_toks,
                        stop_reason: stop,
                        duration_ms: llm_duration_ms,
                    },
                )
                .await;
            }

            if let Some(ref usage) = response.usage {
                usage_totals.record(usage);
            }

            if response.tool_calls.is_empty() {
                // Emit Done before finalising so the REPL can clear the spinner
                // line before the final response is printed to stdout.
                self.notify(|| AgentProgressEvent::Done);
                let end = TurnEnd {
                    iterations,
                    usage: usage_totals,
                    pre_response_context_tokens: context_tokens,
                    current_turn,
                };
                return Ok(self.finalize_text_response(messages, response, end).await);
            }

            let appended_from = messages.len();
            self.execute_tool_calls_for_response(messages, current_turn, response)
                .await;
            // Stream this turn's output (assistant message + tool results) over
            // the live progress path so a parent/inspector sees it turn-by-turn,
            // not only at completion (#797). The clone is only paid when a
            // progress callback is registered (via `notify`'s guard), and the
            // Arc<[Message]> payload makes further event clones refcount bumps (#993).
            self.notify(|| AgentProgressEvent::TurnCompleted {
                messages: messages[appended_from..].into(),
            });
            // Tool calls were executed and spilled — mark dirty for next iteration
            spills_dirty = self.spill_store.is_some();
            iterations += 1;
            current_turn += 1;

            if iterations >= self.max_tool_iterations {
                // Emit Done so the spinner is cleared before the limit message.
                self.notify(|| AgentProgressEvent::Done);
                return Ok(AgentResult {
                    response: format!(
                        "Tool iteration limit ({}) reached. Stopping.",
                        self.max_tool_iterations
                    ),
                    tool_iterations: iterations,
                    iteration_limit_reached: true,
                    input_tokens: usage_totals.context_input_tokens,
                    context_tokens: context_pruning::estimate_total_tokens(messages),
                    output_tokens: usage_totals.output_tokens,
                    billed_input_tokens: usage_totals.billed_input_tokens,
                    billed_output_tokens: usage_totals.billed_output_tokens,
                    cache_read_tokens: usage_totals.cache_read_tokens,
                    cache_write_tokens: usage_totals.cache_write_tokens,
                    cost_micro_usd: usage_totals.cost_micro_usd,
                });
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
            tool_count: self.tool_registry.tool_count(),
        }
    }
}

#[cfg(test)]
#[path = "agent_loop_993_tests.rs"]
mod issue_993_tests;
#[cfg(test)]
#[path = "agent_loop_spill_tests.rs"]
mod spill_tests;
#[cfg(test)]
#[path = "agent_loop_swap_tests.rs"]
mod swap_tests;
#[cfg(test)]
#[path = "agent_loop_tests.rs"]
mod tests;
