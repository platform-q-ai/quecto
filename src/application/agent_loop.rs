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
use crate::domain::provider_error::classify_provider_error;
use crate::domain::session::{ContextSpillStore, SpillEntry};
use crate::domain::tool::ToolRegistry;

#[path = "agent_loop_pruning.rs"]
mod agent_loop_pruning;

const DEFAULT_MAX_TOOL_ITERATIONS: u32 = 999_999;
const MAX_PROVIDER_ATTEMPTS: usize = 3;
const PROVIDER_RETRY_BACKOFF_MS: u64 = 100;

/// Configuration for building an agent loop.
pub struct AgentLoopConfig {
    pub provider: Arc<dyn LlmProvider>,
    pub tool_registry: Box<dyn ToolRegistry>,
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub spill_store: Option<Arc<dyn ContextSpillStore>>,
    pub session_key: String,
    pub context_collapse_after_turns: u32,
    pub max_context_tokens: usize,
    /// Optional callback to receive live progress events during agent processing.
    /// Used by the REPL progress renderer to display tool activity to the user.
    /// Pass `None` for headless operation (no-op, zero overhead).
    pub progress_callback: Option<ProgressCallback>,
    /// When `true`, use `chat_stream_incremental()` for LLM calls so that
    /// `AgentProgressEvent::Token` events are emitted in real time.
    /// Set by the UDS agent path; `false` for REPL (which uses
    /// non-streaming mock servers in tests).
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
}

pub struct AgentLoopImpl {
    provider: Arc<dyn LlmProvider>,
    tool_registry: Box<dyn ToolRegistry>,
    model: String,
    max_tokens: u32,
    temperature: f32,
    max_tool_iterations: u32,
    skill_count: usize,
    spill_store: Option<Arc<dyn ContextSpillStore>>,
    session_key: String,
    context_collapse_after_turns: u32,
    max_context_tokens: usize,
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

/// Arguments for building a tool result message (avoids clippy 5-arg limit).
struct ToolMessageArgs<'a> {
    tc: &'a ToolCall,
    content: String,
    image_blocks: Vec<crate::domain::tool::ImageBlock>,
    spill_id: String,
    is_error: bool,
}

struct StreamProviderError {
    error: DomainError,
    emitted_event: bool,
}

impl AgentLoopImpl {
    pub fn new(config: AgentLoopConfig) -> Self {
        Self {
            provider: config.provider,
            tool_registry: config.tool_registry,
            model: config.model.clone(),
            max_tokens: config.max_tokens,
            temperature: config.temperature,
            max_tool_iterations: DEFAULT_MAX_TOOL_ITERATIONS,
            skill_count: 0,
            spill_store: config.spill_store,
            session_key: config.session_key,
            context_collapse_after_turns: config.context_collapse_after_turns,
            max_context_tokens: config.max_context_tokens,
            progress_callback: config.progress_callback,
            streaming: config.streaming,
            effort: config.effort,
            system_prompt_provider: config.system_prompt_provider,
            audit_log: config.audit_log,
        }
    }

    /// Switch the model used for all subsequent LLM calls.
    pub fn set_model(&mut self, model: String) {
        self.model = model;
    }

    /// Replace the LLM provider after config reload.
    pub fn swap_provider(&mut self, provider: Arc<dyn LlmProvider>) {
        self.provider = provider;
    }

    /// Return the currently configured model name.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Context-window ceiling (tokens), surfaced for UDS clients.
    pub fn max_context_tokens(&self) -> usize {
        self.max_context_tokens
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

    /// Return names of tools registered from extensions.
    ///
    /// Used by UDS `get_extensions` to report only tools that are actually
    /// available (shadows are rejected during registration).
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

    /// Set or replace the progress callback at runtime.
    ///
    /// Used by the UDS agent to install a streaming-token forwarder after
    /// construction.  Pass `None` to clear the callback.
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

    /// Set or replace the audit log at runtime.
    ///
    /// Used by the UDS entry point to wire the audit log after construction.
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

    /// Set the skill count (for startup info).
    pub fn with_skill_count(mut self, count: usize) -> Self {
        self.skill_count = count;
        self
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
            first.content = prompt;
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
            max_tokens: self.max_tokens,
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
        // Move content out (no clone). Clone tool_calls once — needed because
        // Message::assistant takes ownership but we push tool results to messages
        // below, requiring mutable access that conflicts with borrowing back.
        let content = response.content.unwrap_or_default();
        let tool_calls = response.tool_calls;
        messages.push(Message::assistant(content, tool_calls.clone()));

        for (idx, tc) in tool_calls.iter().enumerate() {
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
                spill_id,
                is_error,
            });
            tool_msg.turn = Some(current_turn);
            self.spill_tool_message(&mut tool_msg).await;
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
        // Cap result_content for the progress event to avoid cloning huge strings.
        // The TUI only previews the first ~10 lines anyway.
        const MAX_RESULT_EVENT_BYTES: usize = 50 * 1024;
        let result_preview = if content.len() > MAX_RESULT_EVENT_BYTES {
            content[..MAX_RESULT_EVENT_BYTES].to_string()
        } else {
            content.clone()
        };
        self.notify(|| AgentProgressEvent::ToolFinished {
            tool_call_id: tc.id.clone(),
            name: tc.name.clone(),
            arguments: tc.arguments.clone(),
            result_content: result_preview.clone(),
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

    fn build_tool_message(&self, args: ToolMessageArgs) -> Message {
        let mut tool_msg = Message::tool(args.tc.id.clone(), args.content);
        tool_msg.tool_name = Some(args.tc.name.clone());
        tool_msg.input_preview =
            Some(context_pruning::truncate_utf8_safe(&args.tc.arguments, 100).into_owned());
        tool_msg.spill_id = Some(args.spill_id);
        tool_msg.image_blocks = args.image_blocks;
        tool_msg.is_error = args.is_error;
        tool_msg
    }

    async fn spill_tool_message(&self, tool_msg: &mut Message) {
        let Some(ref spill_store) = self.spill_store else {
            return;
        };

        // Take content out of the message to avoid cloning up to 1MB of tool output.
        // The content is moved into the SpillEntry, used for the append (which borrows),
        // then moved back into the message.
        let content = std::mem::take(&mut tool_msg.content);
        let entry = SpillEntry {
            id: tool_msg.spill_id.clone().unwrap_or_default(),
            tool: tool_msg
                .tool_name
                .clone()
                .unwrap_or_else(|| "tool".to_string()),
            input_preview: tool_msg.input_preview.clone().unwrap_or_default(),
            tokens: context_pruning::estimate_tokens(&content),
            content,
        };
        if let Err(e) = spill_store.append(&self.session_key, &entry).await {
            tracing::warn!(target: "context_prune", error = %e, "failed to spill tool output");
        }
        // Restore content back into the message (entry is consumed here).
        tool_msg.content = entry.content;
    }

    fn finalize_text_response(
        messages: &mut Vec<Message>,
        response: LlmResponse,
        iterations: u32,
        usage: UsageTotals,
    ) -> AgentResult {
        let text = response.content.unwrap_or_default();
        messages.push(Message::assistant(text.clone(), vec![]));
        let context_tokens = if usage.context_input_tokens > 0 {
            usage.context_input_tokens as usize
        } else {
            context_pruning::estimate_total_tokens(messages)
        };
        AgentResult {
            response: text,
            tool_iterations: iterations,
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
        for attempt in 1..=MAX_PROVIDER_ATTEMPTS {
            let result = if self.streaming {
                match self.stream_chat_once(request.clone()).await {
                    Ok(response) => Ok(response),
                    Err(stream_error) if stream_error.emitted_event => {
                        // Once the provider has emitted any stream content,
                        // replaying the request would duplicate/corrupt output.
                        return Err(enhance_provider_error(stream_error.error));
                    }
                    Err(stream_error) => Err(stream_error.error),
                }
            } else {
                self.provider.chat(request.clone()).await
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
                        "retrying provider request after transient failure"
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
        // Track whether spills happened so we only rebuild manifest when needed.
        // Start true to build initial manifest from any prior session spills.
        let mut spills_dirty = true;
        let mut usage_totals = UsageTotals::default();

        loop {
            self.refresh_dynamic_system_prompt(messages);
            let context_tokens = self
                .apply_context_pruning(messages, current_turn, spills_dirty)
                .await;

            // Emit Thinking before every LLM call so the REPL spinner activates
            // immediately, including during multi-turn tool loops.
            self.notify(|| AgentProgressEvent::Thinking {
                context_tokens,
                max_context_tokens: self.max_context_tokens,
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
            // Use streaming when enabled (UDS mode) so token events are
            // forwarded in real time.  REPL/one-shot use the
            // non-streaming path.
            let response = self.call_provider_with_retries(request).await;

            let llm_duration_ms = llm_start.elapsed().as_millis() as u64;
            let response = match response {
                Ok(r) => r,
                Err(e) => {
                    // Audit: Error on provider failure
                    self.audit(
                        current_turn,
                        AuditEvent::Error {
                            source: "provider".into(),
                            tool: None,
                            message: e.to_string(),
                        },
                    )
                    .await;
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
                    .map(|s| match s {
                        crate::domain::message::StopReason::EndTurn => "end_turn",
                        crate::domain::message::StopReason::MaxTokens => "max_tokens",
                        crate::domain::message::StopReason::ToolUse => "tool_use",
                        crate::domain::message::StopReason::Refusal => "refusal",
                        crate::domain::message::StopReason::Error => "error",
                        crate::domain::message::StopReason::Aborted => "aborted",
                        crate::domain::message::StopReason::Unknown(s) => s.as_str(),
                    })
                    .unwrap_or("unknown")
                    .to_string();
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
                return Ok(Self::finalize_text_response(
                    messages,
                    response,
                    iterations,
                    usage_totals,
                ));
            }

            self.execute_tool_calls_for_response(messages, current_turn, response)
                .await;
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

fn enhance_provider_error(err: DomainError) -> DomainError {
    let DomainError::Provider(message) = err else {
        return err;
    };

    if is_context_or_output_limit_error(&message)
        && !message
            .to_ascii_lowercase()
            .contains("context/output limit")
    {
        return DomainError::Provider(format!(
            "{message}\n\nContext/output limit: the provider rejected the request because the prompt plus requested output appears to exceed a model limit. Try reducing prompt history, lowering max output tokens, or enabling/prioritizing context pruning before retrying."
        ));
    }

    DomainError::Provider(message)
}

fn is_context_or_output_limit_error(message: &str) -> bool {
    let lowered = message.to_ascii_lowercase();
    (lowered.contains("maximum context length")
        || lowered.contains("context length")
        || lowered.contains("context window")
        || lowered.contains("too many tokens")
        || lowered.contains("max_tokens")
        || lowered.contains("max output")
        || lowered.contains("requested") && lowered.contains("tokens"))
        && (lowered.contains("token") || lowered.contains("context"))
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
            skill_count: self.skill_count,
        }
    }
}

#[cfg(test)]
#[path = "agent_loop_spill_tests.rs"]
mod spill_tests;
#[cfg(test)]
#[path = "agent_loop_swap_tests.rs"]
mod swap_tests;
#[cfg(test)]
#[path = "agent_loop_tests.rs"]
mod tests;
