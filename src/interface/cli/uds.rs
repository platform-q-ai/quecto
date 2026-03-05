/// UDS agent loop — headless operation via JSON-lines protocol over a Unix domain socket.
///
/// Entry point: `run_uds_loop` — called from `cmd_agent` when `--mode uds` is set.
/// Session state, statistics, and the public parse helper live in `uds_session`.
use crate::application::agent_loop::AgentLoopImpl;
use crate::domain::agent::AgentLoop;
use crate::domain::message::{Message, Role};
use crate::domain::session::{Session, SessionStore};
use crate::infrastructure::persistence::session_store::FileSessionStore;

use super::protocol::{
    AgentCommand, AgentEvent, StreamingBehavior, ToolResultContent, TurnMessage,
};
use super::uds_session::{
    AgentSession, compute_session_stats, message_to_json, messages_tail_json,
};

pub use super::uds_session::parse_command_line;

// ─── UDS loop ────────────────────────────────────────────────────────────────

/// Arguments for running the UDS loop (avoids long parameter lists).
pub struct UdsLoopArgs<'a> {
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
    /// For long-lived UDS sessions (hours) the timestamp will be stale.
    /// A future improvement would re-inject on each prompt command.
    pub system_prompt: String,
    /// Path to the Unix domain socket file.
    /// In production: used to bind `UnixListener` and printed to stderr.
    /// In tests: unused (the pre-connected `socket_override` is used instead).
    pub socket_path: std::path::PathBuf,
    /// Pre-connected stream injected by tests instead of accepting from a listener.
    /// `None` = bind `socket_path`, `chmod 0o600`, accept one connection.
    pub socket_override: Option<std::os::unix::net::UnixStream>,
    /// Injected session store for testing.  `None` = use `FileSessionStore`.
    pub session_store_override: Option<Box<dyn SessionStore + 'static>>,
}

/// Run the UDS event loop.
///
/// In production: binds a `UnixListener` on `socket_path`, prints the path
/// to stderr, `chmod 0o600`, accepts one client connection, then processes
/// JSON-lines commands until the client closes the connection.  The socket
/// file is unlinked on exit (both clean and panicked via a drop guard).
///
/// In tests: a pre-connected `std::os::unix::net::UnixStream` is passed via
/// `socket_override`, so no listening or `accept()` occurs.
///
/// Returns an exit code (0 = success, 1 = error).
pub fn run_uds_loop(args: UdsLoopArgs<'_>) -> i32 {
    let rt = match crate::interface::cli::build_tokio_runtime() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("failed to create runtime: {e}");
            return 1;
        }
    };
    rt.block_on(uds_loop_async(args))
}

/// Drop guard that removes the socket file on exit.
struct SocketGuard(std::path::PathBuf);

impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Async body of the UDS loop.
async fn uds_loop_async(args: UdsLoopArgs<'_>) -> i32 {
    use std::os::unix::fs::PermissionsExt;

    let UdsLoopArgs {
        agent,
        base_dir,
        session_key,
        model,
        ephemeral,
        system_prompt,
        socket_path,
        socket_override,
        session_store_override,
    } = args;

    let file_store;
    let session_store: &dyn SessionStore = if let Some(ref s) = session_store_override {
        s.as_ref()
    } else {
        file_store = FileSessionStore::new(base_dir);
        &file_store
    };

    let messages = match load_session(session_store, &session_key, ephemeral).await {
        Ok(m) => m,
        Err(err) => {
            eprintln!("failed to load session: {err}");
            return 1;
        }
    };

    // Build reader/writer from the socket, plus an optional cleanup guard.
    // Test path: pre-connected std UnixStream injected via socket_override.
    // Production path: bind UnixListener, accept one client, return guard.
    let (reader, writer, guard) = if let Some(std_stream) = socket_override {
        std_stream
            .set_nonblocking(true)
            .expect("set_nonblocking failed for test socket");
        let tokio_stream = tokio::net::UnixStream::from_std(std_stream)
            .expect("failed to convert std UnixStream to tokio");
        let (r, w) = tokio::io::split(tokio_stream);
        let r: Box<dyn tokio::io::AsyncRead + Send + Unpin> = Box::new(r);
        let w: Box<dyn tokio::io::AsyncWrite + Send + Unpin> = Box::new(w);
        (r, w, None::<SocketGuard>)
    } else {
        let _ = std::fs::remove_file(&socket_path);
        let listener = match tokio::net::UnixListener::bind(&socket_path) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("failed to bind socket {}: {e}", socket_path.display());
                return 1;
            }
        };
        if let Err(e) =
            std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))
        {
            eprintln!("warning: failed to chmod socket: {e}");
        }
        // Print socket path to stderr — callers poll for this line to discover the path.
        eprintln!("quecto-agent-socket: {}", socket_path.display());
        let stream = match listener.accept().await {
            Ok((s, _)) => s,
            Err(e) => {
                eprintln!("failed to accept connection: {e}");
                return 1;
            }
        };
        let (r, w) = tokio::io::split(stream);
        let r: Box<dyn tokio::io::AsyncRead + Send + Unpin> = Box::new(r);
        let w: Box<dyn tokio::io::AsyncWrite + Send + Unpin> = Box::new(w);
        (r, w, Some(SocketGuard(socket_path)))
    };

    // Run the command loop. `guard` is dropped here after the loop finishes,
    // which unlinks the socket file (production path only).
    uds_loop_with_streams(
        StreamLoopArgs {
            reader,
            writer,
            _guard: guard,
            agent,
            messages,
            agent_session: AgentSession::new(model, session_key.clone()),
            session_key,
            ephemeral,
            system_prompt,
        },
        session_store,
    )
    .await
}

/// Arguments for [`uds_loop_with_streams`] — avoids the clippy too-many-arguments lint.
struct StreamLoopArgs {
    reader: Box<dyn tokio::io::AsyncRead + Send + Unpin>,
    writer: Box<dyn tokio::io::AsyncWrite + Send + Unpin>,
    /// Drop guard that unlinks the socket file on exit (production path only).
    _guard: Option<SocketGuard>,
    agent: AgentLoopImpl,
    messages: Vec<Message>,
    agent_session: AgentSession,
    session_key: String,
    ephemeral: bool,
    system_prompt: String,
}

/// Run the command loop with already-established reader/writer streams.
/// The `_guard` (if present) unlinks the socket file when dropped.
async fn uds_loop_with_streams(args: StreamLoopArgs, session_store: &dyn SessionStore) -> i32 {
    let StreamLoopArgs {
        reader,
        mut writer,
        _guard,
        mut agent,
        mut messages,
        mut agent_session,
        session_key,
        ephemeral,
        system_prompt,
    } = args;

    inject_system_prompt(&mut messages, &system_prompt);

    run_command_loop(
        reader,
        &mut DispatchCtx {
            agent: &mut agent,
            messages: &mut messages,
            session: &mut agent_session,
            stdout: &mut *writer,
            session_key: &session_key,
        },
    )
    .await;

    if !ephemeral && !session_key.is_empty() {
        remove_injected_system_prompt(&mut messages, &system_prompt);
        let session = Session {
            key: session_key,
            messages: std::mem::take(&mut messages),
        };
        let _ = session_store.save(&session).await;
    }

    0 // _guard dropped here — socket file unlinked (production path)
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
            let ev = AgentEvent::err(None, "parse_error", "line exceeds 1 MiB limit");
            emit_event(ctx.stdout, &ev).await;
            continue;
        }
        let line = line.trim().to_owned();
        if line.is_empty() {
            continue;
        }

        let cmd = match parse_command_line(&line) {
            Ok(c) => c,
            Err(e) => {
                let ev = AgentEvent::Response {
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
    session: &'a mut AgentSession,
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
fn query_response_data(cmd: &AgentCommand, ctx: &DispatchCtx<'_>) -> Option<serde_json::Value> {
    match cmd {
        AgentCommand::GetState { .. } => {
            let state = ctx.session.state_snapshot(ctx.messages.len());
            Some(serde_json::to_value(&state).unwrap_or_default())
        }
        AgentCommand::GetMessages { .. } => {
            let msgs: Vec<serde_json::Value> = ctx.messages.iter().map(message_to_json).collect();
            Some(serde_json::json!({ "messages": msgs }))
        }
        AgentCommand::GetMessagesTail { count, .. } => {
            Some(messages_tail_json(ctx.messages, *count))
        }
        AgentCommand::GetSessionStats { .. } => {
            let stats = compute_session_stats(ctx.session_key, ctx.messages);
            Some(serde_json::to_value(&stats).unwrap_or_default())
        }
        _ => None,
    }
}

/// Dispatch a single RPC command.  Returns `true` if the loop should exit.
async fn dispatch_command(cmd: AgentCommand, ctx: &mut DispatchCtx<'_>) -> bool {
    let id = cmd.id().map(str::to_owned);
    let type_name = cmd.type_name().to_owned();

    // Fast path: read-only query commands share the same emit pattern.
    if let Some(data) = query_response_data(&cmd, ctx) {
        let ev = AgentEvent::ok(id.as_deref(), &type_name, Some(data));
        emit_event(ctx.stdout, &ev).await;
        return false;
    }

    match cmd {
        AgentCommand::Prompt {
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
        AgentCommand::Steer { message, .. } | AgentCommand::FollowUp { message, .. } => {
            ctx.session.enqueue_pending(message);
            let ev = AgentEvent::ok(id.as_deref(), &type_name, None);
            emit_event(ctx.stdout, &ev).await;
            false
        }

        // Abort is a no-op when the agent is idle (sequential loop: a command can
        // only arrive between prompts). Abort-during-streaming requires a CancelFlag
        // wired into the agent process loop — deferred to follow-up.
        AgentCommand::Abort { .. } => {
            let ev = AgentEvent::ok(id.as_deref(), &type_name, None);
            emit_event(ctx.stdout, &ev).await;
            false
        }

        AgentCommand::SetModel {
            model,
            provider,
            model_id,
            ..
        } => {
            let resolved_model = match resolve_set_model_target(model, provider, model_id) {
                Ok(m) => m,
                Err(msg) => {
                    let ev = AgentEvent::err(id.as_deref(), &type_name, msg);
                    emit_event(ctx.stdout, &ev).await;
                    return false;
                }
            };

            ctx.agent.set_model(resolved_model.clone());
            ctx.session.set_model(resolved_model);
            tracing::debug!(new_model = %ctx.session.model(), "UDS: model switched");
            let ev = AgentEvent::ok(id.as_deref(), &type_name, None);
            emit_event(ctx.stdout, &ev).await;
            false
        }

        // Query variants are handled by the fast path above; this arm is a safety
        // net in case query_response_data is not updated when a new query variant is
        // added. It emits a graceful error rather than panicking.
        AgentCommand::GetState { .. }
        | AgentCommand::GetMessages { .. }
        | AgentCommand::GetMessagesTail { .. }
        | AgentCommand::GetSessionStats { .. } => {
            tracing::error!(command = %type_name, "query variant reached dispatch fallback — update query_response_data");
            let ev = AgentEvent::err(
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
        session: agent_session,
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
    if agent_session.is_streaming() {
        match streaming_behavior {
            Some(StreamingBehavior::FollowUp) | Some(StreamingBehavior::Steer) => {
                agent_session.enqueue_pending(message);
                let ev = AgentEvent::ok(id.as_deref(), &type_name, None);
                emit_event(stdout, &ev).await;
                return false;
            }
            None => {
                let ev = AgentEvent::err(
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
        session: agent_session,
        stdout,
        message,
    })
    .await;

    let ev = AgentEvent::ok(id.as_deref(), &type_name, None);
    emit_event(stdout, &ev).await;

    // Drain pending follow-ups.
    loop {
        let pending = agent_session.drain_pending();
        if pending.is_empty() {
            break;
        }
        for follow_msg in pending {
            run_agent_prompt(PromptArgs {
                agent,
                messages,
                session: agent_session,
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
    session: &'a mut AgentSession,
    stdout: &'a mut (dyn tokio::io::AsyncWrite + Send + Unpin),
    message: String,
}

/// Run a single agent prompt, emitting UDS events.  Returns 0 on success.
async fn run_agent_prompt(args: PromptArgs<'_>) -> i32 {
    let PromptArgs {
        agent,
        messages,
        session: agent_session,
        stdout,
        message,
    } = args;

    agent_session.set_streaming(true);
    emit_event(stdout, &AgentEvent::AgentStart).await;
    emit_event(stdout, &AgentEvent::TurnStart).await;

    messages.push(Message::user(message));

    let before_len = messages.len();
    let result = agent.process(messages).await;
    agent_session.set_streaming(false);

    match result {
        Ok(agent_result) => {
            emit_tool_events_from_messages(stdout, &messages[before_len..]).await;

            let turn_end = AgentEvent::TurnEnd {
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
            emit_event(stdout, &AgentEvent::AgentEnd { messages: run_msgs }).await;
            0
        }
        Err(e) => {
            emit_event(
                stdout,
                &AgentEvent::err(None, "agent_error", format!("{e}")),
            )
            .await;
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
                    &AgentEvent::ToolExecutionStart {
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
                &AgentEvent::ToolExecutionEnd {
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

/// Write an event as a JSON line followed by a newline.
async fn emit_event(writer: &mut (dyn tokio::io::AsyncWrite + Send + Unpin), event: &AgentEvent) {
    use tokio::io::AsyncWriteExt;
    let line = event.to_json_line() + "\n";
    let _ = writer.write_all(line.as_bytes()).await;
}

#[cfg(test)]
#[path = "uds_tests.rs"]
mod tests;
