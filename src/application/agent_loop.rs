// Agent loop implementation: orchestrates LLM calls and tool execution.
// Depends on: domain::LlmProvider, domain::Tool, infrastructure::tools::ToolRegistry

use std::pin::Pin;
use std::sync::Arc;

use crate::application::context_pruning;
use crate::domain::agent::{
    AgentInfo, AgentLoop, AgentProgressEvent, AgentResult, ProgressCallback,
};
use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, Message, ToolCall};
use crate::domain::provider::{ChatRequest, LlmProvider, StreamEvent};
use crate::domain::session::{ContextSpillStore, SpillEntry};
use crate::domain::tool::ToolRegistry;

/// Default maximum tool iterations before the loop is forcibly stopped.
const DEFAULT_MAX_TOOL_ITERATIONS: u32 = 999_999;

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
}

/// Concrete implementation of the agent loop.
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
        }
    }

    /// Switch the model used for all subsequent LLM calls.
    ///
    /// The model name is validated only by the underlying provider at call time.
    pub fn set_model(&mut self, model: String) {
        self.model = model;
    }

    /// Return the currently configured model name.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Fire a progress event to the registered callback, if any.
    ///
    /// Accepts a closure that constructs the event so it is only evaluated
    /// when a callback is actually registered. On the headless path
    /// (`progress_callback = None`) the closure is never called — no String
    /// allocations, no truncation scans. This keeps the hot tool-execution
    /// path zero-cost when progress reporting is disabled.
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

    /// Set the skill count (for startup info).
    pub fn with_skill_count(mut self, count: usize) -> Self {
        self.skill_count = count;
        self
    }

    async fn apply_context_pruning(
        &self,
        messages: &mut Vec<Message>,
        current_turn: u32,
        spills_dirty: bool,
    ) {
        // Collapse is disabled by default (COLLAPSE_DISABLED = u32::MAX).
        // Still available for users who explicitly lower the config value.
        let collapsed = if self.context_collapse_after_turns < context_pruning::COLLAPSE_DISABLED {
            context_pruning::collapse_old_tool_results(
                messages,
                current_turn,
                self.context_collapse_after_turns,
            )
        } else {
            0
        };
        let dropped = context_pruning::enforce_context_ceiling(messages, self.max_context_tokens);
        // Only rebuild manifest when spills have changed (new tool results spilled)
        if spills_dirty {
            if let Some(ref spill_store) = self.spill_store {
                context_pruning::update_spill_manifest(
                    messages,
                    spill_store.as_ref(),
                    &self.session_key,
                )
                .await;
            }
        }
        if collapsed > 0 || dropped > 0 {
            tracing::info!(
                target: "context_prune",
                collapsed,
                dropped,
                turn = current_turn,
                total_tokens = context_pruning::estimate_total_tokens(messages),
                "context pruned"
            );
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
            Some(self.session_key.clone())
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
            let (content, image_blocks, is_error) = self.execute_single_tool_call(tc).await;
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
        self.notify(|| AgentProgressEvent::ToolFinished {
            name: tc.name.clone(),
            arguments: tc.arguments.clone(),
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
        tool_msg.input_preview = Some(context_pruning::truncate_utf8_safe(&args.tc.arguments, 100));
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
        &self,
        messages: &mut Vec<Message>,
        response: LlmResponse,
        iterations: u32,
    ) -> AgentResult {
        let text = response.content.unwrap_or_default();
        messages.push(Message::assistant(text.clone(), vec![]));
        AgentResult {
            response: text,
            tool_iterations: iterations,
            iteration_limit_reached: false,
        }
    }

    /// Send a chat request using incremental streaming.
    ///
    /// Emits `AgentProgressEvent::Token` for each text delta so the UDS layer
    /// can forward them as `{"type":"token"}` events.  Falls back gracefully
    /// for providers whose `chat_stream_incremental()` wraps `chat()` (emitting
    /// only a single `Done`).
    async fn stream_chat(&self, request: ChatRequest<'_>) -> Result<LlmResponse, DomainError> {
        let mut rx = self.provider.chat_stream_incremental(request).await;
        while let Some(event) = rx.recv().await {
            match event {
                StreamEvent::TextDelta(t) => {
                    self.notify(|| AgentProgressEvent::Token(t));
                }
                StreamEvent::Done(response) => return Ok(response),
                StreamEvent::Error(e) => {
                    return Err(DomainError::Provider(e));
                }
                // Tool call streaming events are handled by the provider's
                // accumulator — they assemble into LlmResponse.tool_calls
                // and are delivered via StreamEvent::Done.
                _ => {}
            }
        }
        // Channel closed without Done — shouldn't happen but handle gracefully.
        Err(DomainError::Provider(
            "streaming channel closed without completion".to_string(),
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

        loop {
            self.apply_context_pruning(messages, current_turn, spills_dirty)
                .await;

            // Emit Thinking before every LLM call so the REPL spinner activates
            // immediately, including during multi-turn tool loops.
            let context_tokens = context_pruning::estimate_total_tokens(messages);
            let provider = self.provider.name().to_string();
            let model = self.model.clone();
            let max_context_tokens = self.max_context_tokens;
            self.notify(|| AgentProgressEvent::Thinking {
                context_tokens,
                max_context_tokens,
                provider,
                model,
            });

            let request = self.build_chat_request(messages, tool_defs);
            // Use streaming when enabled (UDS mode) so token events are
            // forwarded in real time.  REPL/one-shot use the
            // non-streaming path.
            let response = if self.streaming {
                self.stream_chat(request).await
            } else {
                self.provider.chat(request).await
            };
            let response = match response {
                Ok(r) => r,
                Err(e) => {
                    self.notify(|| AgentProgressEvent::Done);
                    return Err(e);
                }
            };

            if response.tool_calls.is_empty() {
                // Emit Done before finalising so the REPL can clear the spinner
                // line before the final response is printed to stdout.
                self.notify(|| AgentProgressEvent::Done);
                return Ok(self.finalize_text_response(messages, response, iterations));
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
            skill_count: self.skill_count,
        }
    }
}

#[cfg(test)]
#[path = "agent_loop_tests.rs"]
mod tests;
