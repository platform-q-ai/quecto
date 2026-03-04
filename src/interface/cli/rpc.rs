/// RPC agent loop — headless operation via JSON-lines protocol over stdin/stdout.
///
/// Entry point: `run_rpc_loop` — called from `cmd_agent` when `--mode rpc` is set.
use crate::application::agent_loop::AgentLoopImpl;
use crate::domain::agent::AgentLoop;
use crate::domain::message::{Message, Role};
use crate::domain::session::{Session, SessionStore};
use crate::infrastructure::persistence::session_store::FileSessionStore;

use super::rpc_types::{
    RpcCommand, RpcEvent, SessionState, SessionStats, StreamingBehavior, TokenStats,
    ToolResultContent, TurnMessage,
};

// ─── Public parse helper (used by unit tests) ────────────────────────────────

/// Parse a single JSON line into an `RpcCommand`.  Returns `Err` for invalid
/// JSON or an unrecognised command type.
pub fn parse_rpc_line(line: &str) -> Result<RpcCommand, String> {
    if line.trim().is_empty() {
        return Err("empty line".to_string());
    }
    serde_json::from_str(line).map_err(|e| format!("parse error: {e}"))
}

// ─── Session state tracker ────────────────────────────────────────────────────

/// In-memory state for an active RPC session.
pub struct RpcSession {
    model: String,
    session_key: String,
    streaming: bool,
    pending: Vec<String>,
}

impl RpcSession {
    pub fn new(model: String, session_key: String) -> Self {
        Self {
            model,
            session_key,
            streaming: false,
            pending: Vec::new(),
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn is_streaming(&self) -> bool {
        self.streaming
    }

    pub fn set_model(&mut self, model: String) {
        self.model = model;
    }

    pub fn set_streaming(&mut self, v: bool) {
        self.streaming = v;
    }

    /// Maximum number of pending (steer/follow_up) messages buffered at once.
    /// Prevents OOM from a flood of pending messages from a misbehaving client.
    pub const MAX_PENDING: usize = 64;

    pub fn enqueue_pending(&mut self, msg: String) {
        if self.pending.len() < Self::MAX_PENDING {
            self.pending.push(msg);
        }
        // Silently drop if the queue is full — caller already got a success ack.
    }

    pub fn drain_pending(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending)
    }

    pub fn state_snapshot(&self, message_count: usize) -> SessionState {
        SessionState {
            model: self.model.clone(),
            is_streaming: self.streaming,
            session_key: self.session_key.clone(),
            message_count,
            pending_message_count: self.pending.len(),
        }
    }
}

// ─── Session statistics ───────────────────────────────────────────────────────

/// Compute session statistics from the current message history.
///
/// Note: token counts are not available from `Message` objects (usage is only
/// on `LlmResponse` which is not persisted on `Message`).  The token fields
/// are zeroed for now; a future enhancement can thread usage through the loop.
pub fn compute_session_stats(session_key: &str, messages: &[Message]) -> SessionStats {
    let mut user_messages = 0usize;
    let mut assistant_messages = 0usize;
    let mut tool_calls_count = 0usize;
    let mut tool_results_count = 0usize;

    for msg in messages {
        match msg.role {
            Role::User => user_messages += 1,
            Role::Assistant => {
                assistant_messages += 1;
                tool_calls_count += msg.tool_calls.len();
            }
            Role::Tool => tool_results_count += 1,
            Role::System => {}
        }
    }

    SessionStats {
        session_key: session_key.to_owned(),
        user_messages,
        assistant_messages,
        tool_calls: tool_calls_count,
        tool_results: tool_results_count,
        total_messages: messages.len(),
        tokens: TokenStats::default(),
        cost: 0.0,
    }
}

/// Return a JSON value containing the last `count` messages in chronological order.
pub fn messages_tail_json(messages: &[Message], count: usize) -> serde_json::Value {
    let skip = messages.len().saturating_sub(count);
    let msgs_json: Vec<serde_json::Value> = messages[skip..].iter().map(message_to_json).collect();
    serde_json::json!({ "messages": msgs_json })
}

// ─── RPC loop ────────────────────────────────────────────────────────────────

/// Arguments for running the RPC loop (avoids long parameter lists).
pub struct RpcLoopArgs<'a> {
    pub agent: AgentLoopImpl,
    pub base_dir: &'a std::path::Path,
    pub session_key: String,
    pub model: String,
    pub ephemeral: bool,
    /// System prompt (datetime preamble + skills + user-supplied `--system`).
    /// Built via `build_system_prompt()` — always non-empty (contains at
    /// least the datetime preamble).  Prepended as a transient
    /// `Message::system` before processing; stripped before persisting the
    /// session so it is not double-injected on the next process invocation.
    ///
    /// NOTE: the datetime in the preamble is fixed at process-start time.
    /// For long-lived RPC sessions (hours) the timestamp will be stale.
    /// A future improvement would re-inject on each prompt command.
    pub system_prompt: String,
    /// Injected stdin for testing.  `None` = use real `tokio::io::stdin`.
    pub stdin_override: Option<Box<dyn tokio::io::AsyncRead + Send + Unpin + 'static>>,
    /// Injected stdout writer for testing.  `None` = use real `tokio::io::stdout`.
    pub stdout_override: Option<Box<dyn tokio::io::AsyncWrite + Send + Unpin + 'static>>,
    /// Injected session store for testing.  `None` = use `FileSessionStore`.
    pub session_store_override: Option<Box<dyn SessionStore + 'static>>,
}

/// Run the RPC event loop.  Reads JSON commands from stdin until EOF;
/// emits JSON events to stdout.  Returns the exit code.
pub fn run_rpc_loop(args: RpcLoopArgs<'_>) -> i32 {
    let rt = match crate::interface::cli::build_tokio_runtime() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("failed to create runtime: {e}");
            return 1;
        }
    };
    rt.block_on(rpc_loop_async(args))
}

/// Async body of the RPC loop.
async fn rpc_loop_async(args: RpcLoopArgs<'_>) -> i32 {
    use tokio::io::AsyncWriteExt;

    let RpcLoopArgs {
        agent,
        base_dir,
        session_key,
        model,
        ephemeral,
        system_prompt,
        stdin_override,
        stdout_override,
        session_store_override,
    } = args;

    let mut agent = agent;
    let file_store;
    let session_store: &dyn SessionStore = if let Some(ref s) = session_store_override {
        s.as_ref()
    } else {
        file_store = FileSessionStore::new(base_dir);
        &file_store
    };

    let mut messages = match load_session(session_store, &session_key, ephemeral).await {
        Ok(m) => m,
        Err(err) => {
            if let Some(mut out) = stdout_override {
                let _ = out
                    .write_all(format!("failed to load session: {err}\n").as_bytes())
                    .await;
            }
            return 1;
        }
    };

    // Inject system prompt as a transient leading message (not persisted).
    inject_system_prompt(&mut messages, &system_prompt);

    let mut rpc_session = RpcSession::new(model, session_key.clone());
    let mut stdout: Box<dyn tokio::io::AsyncWrite + Send + Unpin> = match stdout_override {
        Some(w) => w,
        None => Box::new(tokio::io::stdout()),
    };
    let stdin_reader: Box<dyn tokio::io::AsyncRead + Send + Unpin> = match stdin_override {
        Some(r) => r,
        None => Box::new(tokio::io::stdin()),
    };

    run_command_loop(
        stdin_reader,
        &mut DispatchCtx {
            agent: &mut agent,
            messages: &mut messages,
            rpc_session: &mut rpc_session,
            stdout: &mut *stdout,
            session_key: &session_key,
        },
    )
    .await;

    if !ephemeral && !session_key.is_empty() {
        // Strip the transient system prompt before saving so it is not
        // double-injected on the next process invocation.
        remove_injected_system_prompt(&mut messages, &system_prompt);
        let session = Session {
            key: session_key,
            messages: std::mem::take(&mut messages),
        };
        let _ = session_store.save(&session).await;
    }

    0
}

/// Prepend a system message if `prompt` is non-empty and no system message
/// already exists at the front of `messages`.
///
/// The injected message is **transient** — it is added before the command
/// loop runs and stripped before the session is persisted.  Callers must
/// pair each `inject_system_prompt` with a matching
/// `remove_injected_system_prompt` call.
///
/// If the session already has a leading `Role::System` message (e.g. loaded
/// from a previous run that saved one, or a future feature), the new system
/// prompt is silently skipped to preserve the existing context.  Callers
/// that need to override a persisted system message should strip it first.
pub(crate) fn inject_system_prompt(messages: &mut Vec<Message>, prompt: &str) {
    if prompt.is_empty() {
        return;
    }
    // Skip injection if a system message is already present (e.g. loaded from session).
    if messages.first().is_some_and(|m| m.role == Role::System) {
        return;
    }
    messages.insert(0, Message::system(prompt.to_string()));
}

/// Remove the injected system prompt from `messages` before persisting.
///
/// Only removes the first message if it is a system message with content
/// matching `prompt` exactly — never silently removes user-authored system
/// messages from session history.  Uses an exact content comparison so
/// that an in-flight mutation of the first message (e.g. by a buggy tool)
/// does not accidentally delete session content.
pub(crate) fn remove_injected_system_prompt(messages: &mut Vec<Message>, prompt: &str) {
    if prompt.is_empty() {
        return;
    }
    if messages
        .first()
        .is_some_and(|m| m.role == Role::System && m.content == prompt)
    {
        messages.remove(0);
    }
}

/// Read JSON commands from stdin and dispatch them until EOF or a fatal error.
async fn run_command_loop(
    stdin_reader: Box<dyn tokio::io::AsyncRead + Send + Unpin>,
    ctx: &mut DispatchCtx<'_>,
) {
    use tokio::io::{AsyncBufReadExt, BufReader};

    // 1 MiB per line matches the bash tool cap; prevents OOM from unbounded lines.
    const MAX_LINE_BYTES: usize = 1024 * 1024;
    let mut lines = BufReader::new(stdin_reader).lines();

    loop {
        let line = match lines.next_line().await {
            Ok(Some(l)) => l,
            _ => break,
        };
        if line.len() > MAX_LINE_BYTES {
            let ev = RpcEvent::err(None, "parse_error", "line exceeds 1 MiB limit");
            emit_event(ctx.stdout, &ev).await;
            continue;
        }
        let line = line.trim().to_owned();
        if line.is_empty() {
            continue;
        }

        let cmd = match parse_rpc_line(&line) {
            Ok(c) => c,
            Err(e) => {
                let ev = RpcEvent::Response {
                    id: None,
                    command: "parse_error".to_string(),
                    success: false,
                    data: None,
                    error: Some(e),
                };
                emit_event(ctx.stdout, &ev).await;
                continue;
            }
        };

        if dispatch_command(cmd, ctx).await {
            break;
        }
    }
}

/// Load session messages, or return an empty vec for ephemeral/missing sessions.
async fn load_session(
    store: &dyn SessionStore,
    session_key: &str,
    ephemeral: bool,
) -> Result<Vec<Message>, String> {
    if ephemeral || session_key.is_empty() {
        return Ok(Vec::new());
    }
    match store.load(session_key).await {
        Ok(Some(s)) => Ok(s.messages),
        Ok(None) => Ok(Vec::new()),
        Err(e) => Err(e.to_string()),
    }
}

/// Mutable context threaded through each command dispatch.
struct DispatchCtx<'a> {
    agent: &'a mut AgentLoopImpl,
    messages: &'a mut Vec<Message>,
    rpc_session: &'a mut RpcSession,
    stdout: &'a mut (dyn tokio::io::AsyncWrite + Send + Unpin),
    session_key: &'a str,
}

fn resolve_set_model_target(
    model: Option<String>,
    provider: Option<String>,
    model_id: Option<String>,
) -> Result<String, &'static str> {
    if let Some(m) = model.filter(|m| !m.trim().is_empty()) {
        return Ok(m);
    }

    match (provider, model_id) {
        (Some(provider), Some(model_id)) => {
            if provider.trim().is_empty() || model_id.trim().is_empty() {
                Err("set_model requires non-empty model, or non-empty provider+modelId")
            } else {
                Ok(format!("{provider}/{model_id}"))
            }
        }
        _ => Err("set_model requires model, or provider+modelId"),
    }
}

/// Build the response data payload for read-only query commands.
///
/// Returns `Some(data)` when the command is a recognised query, `None` otherwise.
fn query_response_data(cmd: &RpcCommand, ctx: &DispatchCtx<'_>) -> Option<serde_json::Value> {
    match cmd {
        RpcCommand::GetState { .. } => {
            let state = ctx.rpc_session.state_snapshot(ctx.messages.len());
            Some(serde_json::to_value(&state).unwrap_or_default())
        }
        RpcCommand::GetMessages { .. } => {
            let msgs: Vec<serde_json::Value> = ctx.messages.iter().map(message_to_json).collect();
            Some(serde_json::json!({ "messages": msgs }))
        }
        RpcCommand::GetMessagesTail { count, .. } => Some(messages_tail_json(ctx.messages, *count)),
        RpcCommand::GetSessionStats { .. } => {
            let stats = compute_session_stats(ctx.session_key, ctx.messages);
            Some(serde_json::to_value(&stats).unwrap_or_default())
        }
        _ => None,
    }
}

/// Dispatch a single RPC command.  Returns `true` if the loop should exit.
async fn dispatch_command(cmd: RpcCommand, ctx: &mut DispatchCtx<'_>) -> bool {
    let id = cmd.id().map(str::to_owned);
    let type_name = cmd.type_name().to_owned();

    // Fast path: read-only query commands share the same emit pattern.
    if let Some(data) = query_response_data(&cmd, ctx) {
        let ev = RpcEvent::ok(id.as_deref(), &type_name, Some(data));
        emit_event(ctx.stdout, &ev).await;
        return false;
    }

    match cmd {
        RpcCommand::Prompt {
            message,
            streaming_behavior,
            ..
        } => {
            handle_prompt(
                ctx,
                PromptCommand {
                    id,
                    type_name,
                    message,
                    streaming_behavior,
                },
            )
            .await
        }

        // Steer and FollowUp both enqueue a pending message. In the current sequential
        // implementation there is no runtime distinction: the pending queue is drained
        // after each prompt regardless of how messages were enqueued.  True steer
        // semantics (interrupt after current tool) require threading a CancelFlag into
        // the agent loop — deferred to a follow-up PR (#233 MVP).
        RpcCommand::Steer { message, .. } | RpcCommand::FollowUp { message, .. } => {
            ctx.rpc_session.enqueue_pending(message);
            let ev = RpcEvent::ok(id.as_deref(), &type_name, None);
            emit_event(ctx.stdout, &ev).await;
            false
        }

        // Abort is a no-op when the agent is idle (sequential loop: a command can
        // only arrive between prompts). Abort-during-streaming requires a CancelFlag
        // wired into the agent process loop — deferred to follow-up.
        RpcCommand::Abort { .. } => {
            let ev = RpcEvent::ok(id.as_deref(), &type_name, None);
            emit_event(ctx.stdout, &ev).await;
            false
        }

        RpcCommand::SetModel {
            model,
            provider,
            model_id,
            ..
        } => {
            let resolved_model = match resolve_set_model_target(model, provider, model_id) {
                Ok(m) => m,
                Err(msg) => {
                    let ev = RpcEvent::err(id.as_deref(), &type_name, msg);
                    emit_event(ctx.stdout, &ev).await;
                    return false;
                }
            };

            ctx.agent.set_model(resolved_model.clone());
            ctx.rpc_session.set_model(resolved_model);
            tracing::debug!(new_model = %ctx.rpc_session.model(), "RPC: model switched");
            let ev = RpcEvent::ok(id.as_deref(), &type_name, None);
            emit_event(ctx.stdout, &ev).await;
            false
        }

        // Query variants are handled by the fast path above; this arm is a safety
        // net in case query_response_data is not updated when a new query variant is
        // added. It emits a graceful error rather than panicking.
        RpcCommand::GetState { .. }
        | RpcCommand::GetMessages { .. }
        | RpcCommand::GetMessagesTail { .. }
        | RpcCommand::GetSessionStats { .. } => {
            tracing::error!(command = %type_name, "query variant reached dispatch fallback — update query_response_data");
            let ev = RpcEvent::err(
                id.as_deref(),
                &type_name,
                "internal: unhandled query command",
            );
            emit_event(ctx.stdout, &ev).await;
            false
        }
    }
}

/// Arguments for [`handle_prompt`].
struct PromptCommand {
    id: Option<String>,
    type_name: String,
    message: String,
    streaming_behavior: Option<StreamingBehavior>,
}

/// Handle a `prompt` command: run agent, emit events, drain follow-ups.
/// Returns `true` if the loop should exit (agent error).
async fn handle_prompt(ctx: &mut DispatchCtx<'_>, cmd: PromptCommand) -> bool {
    let DispatchCtx {
        agent,
        messages,
        rpc_session,
        stdout,
        ..
    } = ctx;
    let PromptCommand {
        id,
        type_name,
        message,
        streaming_behavior,
    } = cmd;
    // If the agent is currently running, require streaming_behavior.
    if rpc_session.is_streaming() {
        match streaming_behavior {
            Some(StreamingBehavior::FollowUp) | Some(StreamingBehavior::Steer) => {
                rpc_session.enqueue_pending(message);
                let ev = RpcEvent::ok(id.as_deref(), &type_name, None);
                emit_event(stdout, &ev).await;
                return false;
            }
            None => {
                let ev = RpcEvent::err(
                    id.as_deref(),
                    &type_name,
                    "agent is running; provide streamingBehavior",
                );
                emit_event(stdout, &ev).await;
                return false;
            }
        }
    }

    let exit = run_agent_prompt(PromptArgs {
        agent,
        messages,
        rpc_session,
        stdout,
        message,
    })
    .await;

    let ev = RpcEvent::ok(id.as_deref(), &type_name, None);
    emit_event(stdout, &ev).await;

    // Drain pending follow-ups.
    loop {
        let pending = rpc_session.drain_pending();
        if pending.is_empty() {
            break;
        }
        for follow_msg in pending {
            run_agent_prompt(PromptArgs {
                agent,
                messages,
                rpc_session,
                stdout,
                message: follow_msg,
            })
            .await;
        }
    }

    exit != 0
}

// ─── Agent prompt execution ───────────────────────────────────────────────────

/// Arguments for [`run_agent_prompt`] — avoids the clippy too-many-arguments lint.
struct PromptArgs<'a> {
    agent: &'a mut AgentLoopImpl,
    messages: &'a mut Vec<Message>,
    rpc_session: &'a mut RpcSession,
    stdout: &'a mut (dyn tokio::io::AsyncWrite + Send + Unpin),
    message: String,
}

/// Run a single agent prompt, emitting RPC events.  Returns 0 on success.
async fn run_agent_prompt(args: PromptArgs<'_>) -> i32 {
    let PromptArgs {
        agent,
        messages,
        rpc_session,
        stdout,
        message,
    } = args;

    rpc_session.set_streaming(true);
    emit_event(stdout, &RpcEvent::AgentStart).await;
    emit_event(stdout, &RpcEvent::TurnStart).await;

    messages.push(Message::user(message));

    let before_len = messages.len();
    let result = agent.process(messages).await;
    rpc_session.set_streaming(false);

    match result {
        Ok(agent_result) => {
            emit_tool_events_from_messages(stdout, &messages[before_len..]).await;

            let turn_end = RpcEvent::TurnEnd {
                message: TurnMessage {
                    role: "assistant".to_string(),
                    content: agent_result.response.clone(),
                    usage: None,
                    stop_reason: None,
                },
                tool_results: vec![],
            };
            emit_event(stdout, &turn_end).await;

            let run_msgs: Vec<serde_json::Value> =
                messages[before_len..].iter().map(message_to_json).collect();
            emit_event(stdout, &RpcEvent::AgentEnd { messages: run_msgs }).await;
            0
        }
        Err(e) => {
            emit_event(stdout, &RpcEvent::err(None, "agent_error", format!("{e}"))).await;
            1
        }
    }
}

// ─── Tool event emission ──────────────────────────────────────────────────────

/// Emit `tool_execution_start` / `tool_execution_end` events from message diff.
async fn emit_tool_events_from_messages(
    stdout: &mut (dyn tokio::io::AsyncWrite + Send + Unpin),
    messages: &[Message],
) {
    for msg in messages {
        if msg.role == Role::Assistant {
            for tc in &msg.tool_calls {
                let args: serde_json::Value =
                    serde_json::from_str(&tc.arguments).unwrap_or_default();
                emit_event(
                    stdout,
                    &RpcEvent::ToolExecutionStart {
                        tool_call_id: tc.id.clone(),
                        tool_name: tc.name.clone(),
                        args,
                    },
                )
                .await;
            }
        } else if msg.role == Role::Tool {
            emit_event(
                stdout,
                &RpcEvent::ToolExecutionEnd {
                    tool_call_id: msg.tool_call_id.clone().unwrap_or_default(),
                    tool_name: msg.tool_name.clone().unwrap_or_default(),
                    result: ToolResultContent {
                        content: vec![serde_json::json!({"type":"text","text": msg.content})],
                    },
                    is_error: msg.is_error,
                },
            )
            .await;
        }
    }
}

/// Serialize a `Message` to a JSON value for protocol emission.
fn message_to_json(msg: &Message) -> serde_json::Value {
    serde_json::json!({
        "role": format!("{:?}", msg.role).to_lowercase(),
        "content": msg.content,
        "toolCalls": msg.tool_calls.iter().map(|tc| serde_json::json!({
            "id": tc.id,
            "name": tc.name,
            "arguments": tc.arguments,
        })).collect::<Vec<_>>(),
        "toolCallId": msg.tool_call_id,
        "toolName": msg.tool_name,
    })
}

/// Write an event as a JSON line followed by a newline.
async fn emit_event(writer: &mut (dyn tokio::io::AsyncWrite + Send + Unpin), event: &RpcEvent) {
    use tokio::io::AsyncWriteExt;
    let line = event.to_json_line() + "\n";
    let _ = writer.write_all(line.as_bytes()).await;
}

#[cfg(test)]
#[path = "rpc_tests.rs"]
mod tests;
