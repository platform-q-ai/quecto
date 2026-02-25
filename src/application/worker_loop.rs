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

use std::sync::{Arc, Mutex};

use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::domain::agent::AgentLoop;
use crate::domain::error::DomainError;
use crate::domain::message::Message;
use crate::domain::provider::LlmProvider;
use crate::domain::tool::{ToolDefinition, ToolRegistry, ToolResult};

// ── Event sink trait ───────────────────────────────────────────────────

/// Port for emitting structured worker events.
///
/// The application layer calls `emit()` to produce events. Infrastructure
/// provides a concrete implementation (e.g. JSON Lines to stdout).
pub trait WorkerEventSink: Send {
    /// Emit an event with the given type and JSON payload.
    /// Returns the sequence number on success.
    fn emit(&mut self, event_type: &str, payload: serde_json::Value) -> Result<u64, String>;
}

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
pub fn build_worker_system_prompt(goal: &str) -> String {
    format!(
        "You are a coding worker executing a task inside a sandboxed environment.\n\
         \n\
         Your goal: {goal}\n\
         \n\
         You have the following tools available:\n\
         - worker_read: Read files with pagination\n\
         - worker_edit: Edit files by exact string replacement\n\
         - worker_grep: Search for patterns in files\n\
         - worker_find: Find files by glob pattern\n\
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
    sink: Arc<Mutex<dyn WorkerEventSink>>,
}

impl std::fmt::Debug for EventEmittingRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventEmittingRegistry").finish()
    }
}

impl EventEmittingRegistry {
    pub fn new(inner: Box<dyn ToolRegistry>, sink: Arc<Mutex<dyn WorkerEventSink>>) -> Self {
        Self { inner, sink }
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
        let args_preview = truncate_args_preview(arguments);
        let args_owned = arguments.to_string();
        let call_id = generate_call_id(&tool_name);

        // Emit tool.start synchronously before the async block
        emit_event(
            &self.sink,
            "tool.start",
            serde_json::json!({
                "tool": &tool_name,
                "call_id": &call_id,
                "args_preview": &args_preview,
            }),
        );

        Box::pin(async move {
            let start = std::time::Instant::now();
            let result = self.inner.execute(&tool_name, &args_owned).await;
            let duration_ms = start.elapsed().as_millis() as u64;

            // Emit tool.result
            let ok = result.as_ref().map(|r| !r.is_error).unwrap_or(false);
            emit_event(
                &self.sink,
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

/// Truncate tool arguments for the `args_preview` field (max 200 chars).
fn truncate_args_preview(args: &str) -> String {
    if args.len() <= 200 {
        args.to_string()
    } else {
        let mut s = args[..200].to_string();
        s.push_str("...");
        s
    }
}

/// Generate a unique call ID for a tool invocation.
fn generate_call_id(tool_name: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("wc_{tool_name}_{n}")
}

/// Emit an event through the sink, ignoring errors.
fn emit_event(sink: &Mutex<dyn WorkerEventSink>, event_type: &str, payload: serde_json::Value) {
    if let Ok(mut s) = sink.lock() {
        let _ = s.emit(event_type, payload);
    }
}

// ── Worker loop runner ─────────────────────────────────────────────────

/// Parameters for running the worker loop (keeps arg count under 4).
pub struct WorkerLoopParams {
    pub config: WorkerLoopConfig,
    pub provider: Arc<dyn LlmProvider>,
    pub sink: Arc<Mutex<dyn WorkerEventSink>>,
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
        sink,
        "log.message",
        serde_json::json!({"level": "info", "message": "worker ready"}),
    );

    // Build system prompt
    let system_prompt = build_worker_system_prompt(&config.goal);

    // Build agent loop
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: params.provider.clone(),
        tool_registry: Box::new(registry),
        model: config.model.clone(),
        max_tokens: config.max_tokens,
        temperature: config.temperature,
        spill_store: None,
        session_key: String::new(),
        context_collapse_after_turns: 3,
        max_context_tokens: 100_000,
    })
    .with_max_tool_iterations(config.max_iterations);

    // Build messages
    let mut messages = vec![Message::system(&system_prompt), Message::user(&config.goal)];

    // Run the agent loop
    match agent.process(&mut messages).await {
        Ok(result) => {
            let msg = if result.iteration_limit_reached {
                format!("worker done (iteration limit reached): {}", result.response)
            } else {
                format!("worker done: {}", result.response)
            };
            emit_event(
                sink,
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
            let msg = format!("worker error: provider returned: {e}");
            emit_event(
                sink,
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
