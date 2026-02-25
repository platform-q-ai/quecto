//! Worker agent loop — runs an LLM agent loop inside the worker process.
//!
//! The worker process receives a goal, builds a tool registry with
//! coding-specific tools, runs the agent loop, and emits structured
//! events via a `WorkerEventSink`. Events include lifecycle messages
//! (ready, done, error) and per-tool-call events (tool.start, tool.result).
//!
//! This module lives in the application layer and depends only on domain
//! types. Infrastructure concerns (concrete emitters, tool registries)
//! are injected by the caller.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::application::context_pruning::truncate_utf8_safe;
use crate::domain::agent::AgentLoop;
use crate::domain::coding_ports::WorkerEventSink;
use crate::domain::error::DomainError;
use crate::domain::message::Message;
use crate::domain::provider::LlmProvider;
use crate::domain::tool::{ToolDefinition, ToolRegistry, ToolResult};

/// Maximum characters for response text in event payloads.
const MAX_EVENT_RESPONSE_CHARS: usize = 500;
/// Maximum characters for error messages in event payloads.
const MAX_EVENT_ERROR_CHARS: usize = 300;
/// Maximum characters for tool argument previews.
const MAX_ARGS_PREVIEW_CHARS: usize = 200;

// ── Worker loop context ────────────────────────────────────────────────

/// Configuration for a worker loop run.
#[derive(Debug, Clone)]
pub struct WorkerLoopConfig {
    pub run_id: String,
    pub job_id: String,
    pub job_dir: String,
    pub goal: String,
    pub model: String,
    pub max_iterations: u32,
    pub max_tokens: u32,
    pub temperature: f32,
}

impl Default for WorkerLoopConfig {
    fn default() -> Self {
        Self {
            run_id: String::new(),
            job_id: String::new(),
            job_dir: String::new(),
            goal: String::new(),
            model: "gpt-4o".to_string(),
            max_iterations: 25,
            max_tokens: 4096,
            temperature: 0.2,
        }
    }
}

/// Result of running the worker loop.
#[derive(Debug)]
pub struct WorkerLoopResult {
    /// Exit code (0 = success, 1 = error).
    pub exit_code: i32,
    /// Final agent response text (if completed successfully).
    pub response: Option<String>,
    /// Whether the iteration limit was reached.
    pub iteration_limit_reached: bool,
}

// ── System prompt builder ──────────────────────────────────────────────

/// Build the system prompt for the worker agent loop.
///
/// Accepts tool definitions so the prompt stays in sync with the actual
/// registry — no hard-coded tool names.
pub fn build_worker_system_prompt(goal: &str, tools: &[ToolDefinition]) -> String {
    let tool_list: String = tools
        .iter()
        .map(|t| format!("- {}: {}", t.name, t.description))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "You are a coding worker executing a task inside a sandboxed environment.\n\
         \n\
         Your goal: {goal}\n\
         \n\
         You have the following tools available:\n\
         {tool_list}\n\
         \n\
         Work autonomously to complete the goal. Read files before editing. \
         Prefer targeted edits over full rewrites. When done, provide a \
         summary of what you did."
    )
}

// ── Event-emitting tool registry ───────────────────────────────────────

/// A tool registry wrapper that emits `tool.start` and `tool.result`
/// events around each tool execution via a `WorkerEventSink`.
pub struct EventEmittingRegistry {
    inner: Box<dyn ToolRegistry>,
    sink: Arc<dyn WorkerEventSink>,
    call_counter: AtomicU64,
}

impl std::fmt::Debug for EventEmittingRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventEmittingRegistry").finish()
    }
}

impl EventEmittingRegistry {
    pub fn new(inner: Box<dyn ToolRegistry>, sink: Arc<dyn WorkerEventSink>) -> Self {
        Self {
            inner,
            sink,
            call_counter: AtomicU64::new(1),
        }
    }

    fn next_call_id(&self, tool_name: &str) -> String {
        let n = self.call_counter.fetch_add(1, Ordering::Relaxed);
        format!("wc_{tool_name}_{n}")
    }
}

impl ToolRegistry for EventEmittingRegistry {
    fn definitions(&self) -> Vec<ToolDefinition> {
        self.inner.definitions()
    }

    fn execute(
        &self,
        name: &str,
        arguments: &str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ToolResult, DomainError>> + Send + '_>,
    > {
        let tool_name = name.to_string();
        let args_preview = truncate_utf8_safe(arguments, MAX_ARGS_PREVIEW_CHARS);
        let call_id = self.next_call_id(&tool_name);

        // Emit tool.start synchronously before the async block
        emit_event(
            self.sink.as_ref(),
            "tool.start",
            serde_json::json!({
                "tool": &tool_name,
                "call_id": &call_id,
                "args_preview": &args_preview,
            }),
        );

        // Pass arguments directly to inner — avoid double allocation
        let args_for_inner = arguments.to_string();

        Box::pin(async move {
            let start = std::time::Instant::now();
            let result = self.inner.execute(&tool_name, &args_for_inner).await;
            let duration_ms = start.elapsed().as_millis() as u64;

            // Emit tool.result
            let ok = result.as_ref().map(|r| !r.is_error).unwrap_or(false);
            emit_event(
                self.sink.as_ref(),
                "tool.result",
                serde_json::json!({
                    "tool": &tool_name,
                    "call_id": &call_id,
                    "ok": ok,
                    "duration_ms": duration_ms,
                }),
            );

            result
        })
    }
}

/// Emit an event through the sink, logging on failure.
fn emit_event(sink: &dyn WorkerEventSink, event_type: &str, payload: serde_json::Value) {
    if let Err(e) = sink.emit(event_type, payload) {
        tracing::warn!(
            event_type = event_type,
            error = %e,
            "failed to emit worker event"
        );
    }
}

// ── Worker loop runner ─────────────────────────────────────────────────

/// Parameters for running the worker loop (keeps arg count under 4).
pub struct WorkerLoopParams {
    pub config: WorkerLoopConfig,
    pub provider: Arc<dyn LlmProvider>,
    pub sink: Arc<dyn WorkerEventSink>,
}

/// Run the worker agent loop.
///
/// The caller provides a tool registry and event sink. This function
/// wraps the registry with event emission, runs the LLM agent loop,
/// and emits lifecycle events (ready, done, error).
pub async fn run_worker_loop(
    params: WorkerLoopParams,
    tool_registry: Box<dyn ToolRegistry>,
) -> WorkerLoopResult {
    let config = &params.config;
    let sink = &params.sink;

    // Wrap with event-emitting registry
    let registry = EventEmittingRegistry::new(tool_registry, sink.clone());

    // Emit ready event
    emit_event(
        sink.as_ref(),
        "log.message",
        serde_json::json!({"level": "info", "message": "worker ready"}),
    );

    // Build system prompt from actual tool definitions
    let tool_defs = registry.definitions();
    let system_prompt = build_worker_system_prompt(&config.goal, &tool_defs);

    // Build agent loop
    let session_key = format!("worker:{}:{}", config.run_id, config.job_id);
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: params.provider.clone(),
        tool_registry: Box::new(registry),
        model: config.model.clone(),
        max_tokens: config.max_tokens,
        temperature: config.temperature,
        spill_store: None,
        session_key,
        context_collapse_after_turns: 3,
        max_context_tokens: 100_000,
    })
    .with_max_tool_iterations(config.max_iterations);

    // Build messages
    let mut messages = vec![Message::system(&system_prompt), Message::user(&config.goal)];

    // Run the agent loop
    match agent.process(&mut messages).await {
        Ok(result) => {
            let preview = truncate_utf8_safe(&result.response, MAX_EVENT_RESPONSE_CHARS);
            let msg = if result.iteration_limit_reached {
                format!("worker done (iteration limit reached): {preview}")
            } else {
                format!("worker done: {preview}")
            };
            emit_event(
                sink.as_ref(),
                "log.message",
                serde_json::json!({"level": "info", "message": msg}),
            );

            WorkerLoopResult {
                exit_code: 0,
                response: Some(result.response),
                iteration_limit_reached: result.iteration_limit_reached,
            }
        }
        Err(e) => {
            let err_text = truncate_utf8_safe(&e.to_string(), MAX_EVENT_ERROR_CHARS);
            let msg = format!("worker error: {err_text}");
            emit_event(
                sink.as_ref(),
                "log.message",
                serde_json::json!({"level": "error", "message": msg}),
            );

            WorkerLoopResult {
                exit_code: 1,
                response: None,
                iteration_limit_reached: false,
            }
        }
    }
}

#[cfg(test)]
#[path = "worker_loop_tests.rs"]
mod tests;
