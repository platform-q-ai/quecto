/// UDS agent loop — headless operation via JSON-lines protocol over a Unix domain socket.
///
/// Entry point: `run_uds_loop` — called from `cmd_agent` when `--mode uds` is set.
/// Session state and statistics live in `uds_session`.
/// Cancellation infrastructure and prompt execution live in `uds_cancel`.
/// Multi-client accept loop and broadcast live in `uds_multi`.
///
/// ## Multi-client architecture (#318)
///
/// The UDS agent accepts multiple simultaneous client connections (Docker-style
/// event bus).  See `uds_multi.rs` for the implementation.
use crate::application::agent_loop::AgentLoopImpl;
use crate::domain::message::{Message, Role};
use crate::domain::session::{Session, SessionStore};
use crate::infrastructure::persistence::session_store::FileSessionStore;

use super::protocol::{AgentCommand, AgentEvent, StreamingBehavior};
use super::uds_cancel::{
    CancelHandle, CancelSlot, PromptArgs, PromptOutcome, arm_cancel, disarm_cancel, fire_cancel,
    run_agent_prompt,
};
use super::uds_multi::{MultiClientArgs, PromptArgsBroadcast, run_agent_prompt_broadcast};
use super::uds_session::{
    AgentSession, compute_session_stats, message_to_json, messages_tail_json,
};

pub use super::protocol::parse_command_line;

// ─── UDS loop ────────────────────────────────────────────────────────────────

/// Arguments for running the UDS loop (avoids long parameter lists).
pub struct UdsLoopArgs<'a> {
    pub agent: AgentLoopImpl,
    pub base_dir: &'a std::path::Path,
    pub session_key: String,
    pub model: String,
    pub ephemeral: bool,
    /// System prompt (datetime preamble + skills + user `--system`).
    pub system_prompt: String,
    /// Path to the Unix domain socket.
    pub socket_path: std::path::PathBuf,
    /// Pre-connected stream injected by tests.
    /// `None` = multi-client mode (production).
    /// `Some` = single-client mode (backward-compatible tests).
    pub socket_override: Option<std::os::unix::net::UnixStream>,
    /// Injected session store for testing.  `None` = use `FileSessionStore`.
    pub session_store_override: Option<Box<dyn SessionStore + 'static>>,
    /// Shared extension registry for get_extensions / reload_extensions.
    pub ext_registry: Option<
        std::sync::Arc<
            std::sync::Mutex<crate::infrastructure::extensions::registry::ExtensionRegistry>,
        >,
    >,
    /// When `Some`, spawn a hot-reload watcher at the given poll interval (multi-client only).
    pub hot_reload_interval: Option<std::time::Duration>,
}

/// Run the UDS event loop.  Returns exit code.
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

/// Remove stale `quecto-agent-*.sock` files older than `max_age` from `dir`.
pub(crate) fn reap_stale_sockets(dir: &std::path::Path, max_age: std::time::Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let cutoff = std::time::SystemTime::now()
        .checked_sub(max_age)
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with("quecto-agent-") || !name_str.ends_with(".sock") {
            continue;
        }
        if let Ok(meta) = entry.metadata() {
            if let Ok(modified) = meta.modified() {
                if modified < cutoff {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }
}

/// Drop guard that removes the socket file on exit.
struct SocketGuard(std::path::PathBuf);

impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Remove stale socket, bind at `path`, apply `chmod 0600`, return listener.
fn bind_secure_socket(path: &std::path::Path) -> std::io::Result<tokio::net::UnixListener> {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::remove_file(path);
    let listener = tokio::net::UnixListener::bind(path)?;
    if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
        let _ = std::fs::remove_file(path);
        return Err(e);
    }
    Ok(listener)
}

/// Async body of the UDS loop.
async fn uds_loop_async(args: UdsLoopArgs<'_>) -> i32 {
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
        ext_registry,
        hot_reload_interval,
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

    if let Some(std_stream) = socket_override {
        // Single-client path: backward-compatible with existing tests.
        single_client_loop(
            SingleClientArgs {
                agent,
                messages,
                model,
                session_key,
                ephemeral,
                system_prompt,
                ext_registry,
            },
            std_stream,
            session_store,
        )
        .await
    } else {
        // Multi-client path: bind, accept loop, broadcast events.
        let listener = match bind_secure_socket(&socket_path) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("failed to bind socket {}: {e}", socket_path.display());
                return 1;
            }
        };
        eprintln!("quecto-agent-socket: {}", socket_path.display());
        let _guard = SocketGuard(socket_path);
        super::uds_multi::multi_client_loop(
            MultiClientArgs {
                agent,
                messages,
                model,
                session_key,
                ephemeral,
                system_prompt,
                ext_registry,
                hot_reload_interval,
            },
            listener,
            session_store,
        )
        .await
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Single-client path (backward-compatible with existing tests)
// ═══════════════════════════════════════════════════════════════════════════════

struct SingleClientArgs {
    agent: AgentLoopImpl,
    messages: Vec<Message>,
    model: String,
    session_key: String,
    ephemeral: bool,
    system_prompt: String,
    ext_registry: Option<
        std::sync::Arc<
            std::sync::Mutex<crate::infrastructure::extensions::registry::ExtensionRegistry>,
        >,
    >,
}

async fn single_client_loop(
    args: SingleClientArgs,
    std_stream: std::os::unix::net::UnixStream,
    session_store: &dyn SessionStore,
) -> i32 {
    let SingleClientArgs {
        agent,
        mut messages,
        model,
        session_key,
        ephemeral,
        system_prompt,
        ext_registry,
    } = args;

    std_stream
        .set_nonblocking(true)
        .expect("set_nonblocking failed for test socket");
    let tokio_stream = tokio::net::UnixStream::from_std(std_stream)
        .expect("failed to convert std UnixStream to tokio");
    let (r, w) = tokio::io::split(tokio_stream);
    let reader: Box<dyn tokio::io::AsyncRead + Send + Unpin> = Box::new(r);
    let mut writer: Box<dyn tokio::io::AsyncWrite + Send + Unpin> = Box::new(w);

    inject_system_prompt(&mut messages, &system_prompt);

    let mut agent_session = AgentSession::new(model, session_key.clone());

    run_command_loop(
        reader,
        &mut DispatchCtx {
            agent: &mut { agent },
            messages: &mut messages,
            session: &mut agent_session,
            stdout: &mut *writer,
            session_key: &session_key,
            cancel_handle: std::sync::Arc::new(std::sync::Mutex::new(CancelSlot::Idle)),
            broadcast_tx: None,
            ext_registry,
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

    0
}

// ═══════════════════════════════════════════════════════════════════════════════
// Shared infrastructure
// ═══════════════════════════════════════════════════════════════════════════════

/// Prepend a transient system message.
pub(crate) fn inject_system_prompt(messages: &mut Vec<Message>, prompt: &str) {
    if prompt.is_empty() {
        return;
    }
    if messages.first().is_some_and(|m| m.role == Role::System) {
        return;
    }
    messages.insert(0, Message::system(prompt.to_string()));
}

/// Remove the injected system prompt before persisting.
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

/// 1 MiB per line cap.
pub(super) const MAX_LINE_BYTES: usize = 1024 * 1024;

/// Cheap substring check for cancel commands.
pub(super) fn is_cancel_command(trimmed: &str) -> bool {
    trimmed.contains("\"type\":\"abort\"") || trimmed.contains("\"type\":\"steer\"")
}

/// Parsed command line or error.
pub(super) enum LineResult {
    Command(AgentCommand),
    ParseError(String),
    LineTooLong,
}

/// Parse a raw text line into a `LineResult`.
pub(super) fn parse_line(line: &str) -> LineResult {
    if line.len() > MAX_LINE_BYTES {
        return LineResult::LineTooLong;
    }
    let line = line.trim();
    if line.is_empty() {
        return LineResult::ParseError(String::new());
    }
    match parse_command_line(line) {
        Ok(c) => LineResult::Command(c),
        Err(e) => LineResult::ParseError(e),
    }
}

/// Read JSON commands from a single-client socket and dispatch them.
async fn run_command_loop(
    reader: Box<dyn tokio::io::AsyncRead + Send + Unpin>,
    ctx: &mut DispatchCtx<'_>,
) {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let cancel_for_reader = std::sync::Arc::clone(&ctx.cancel_handle);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Option<String>>(64);

    let reader_task = tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    if line.len() <= MAX_LINE_BYTES && is_cancel_command(line.trim()) {
                        fire_cancel(&cancel_for_reader);
                    }
                    if tx.send(Some(line)).await.is_err() {
                        break;
                    }
                }
                _ => {
                    let _ = tx.send(None).await;
                    break;
                }
            }
        }
    });

    loop {
        let raw = match rx.recv().await {
            Some(Some(l)) => l,
            _ => break,
        };

        match parse_line(&raw) {
            LineResult::LineTooLong => {
                let ev = AgentEvent::err(None, "parse_error", "line exceeds 1 MiB limit");
                emit_event_to_broadcast_or_writer(ctx, &ev).await;
            }
            LineResult::ParseError(e) if e.is_empty() => {}
            LineResult::ParseError(e) => {
                let ev = AgentEvent::Response {
                    id: None,
                    command: "parse_error".to_string(),
                    success: false,
                    data: None,
                    error: Some(e),
                };
                emit_event_to_broadcast_or_writer(ctx, &ev).await;
            }
            LineResult::Command(cmd) => {
                if dispatch_command(cmd, ctx).await {
                    break;
                }
            }
        }
    }

    reader_task.abort();
}

/// Load session messages.
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
pub(super) struct DispatchCtx<'a> {
    pub agent: &'a mut AgentLoopImpl,
    pub messages: &'a mut Vec<Message>,
    pub session: &'a mut AgentSession,
    pub stdout: &'a mut (dyn tokio::io::AsyncWrite + Send + Unpin),
    pub session_key: &'a str,
    pub cancel_handle: CancelHandle,
    /// `Some` in multi-client mode; `None` in single-client mode.
    pub broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    /// Shared extension registry for get_extensions / reload_extensions.
    /// `None` when extensions are not wired (e.g. legacy tests).
    pub ext_registry: Option<
        std::sync::Arc<
            std::sync::Mutex<crate::infrastructure::extensions::registry::ExtensionRegistry>,
        >,
    >,
}

/// Emit an event: broadcast if available, otherwise write directly.
pub(super) async fn emit_event_to_broadcast_or_writer(
    ctx: &mut DispatchCtx<'_>,
    event: &AgentEvent,
) {
    if let Some(ref tx) = ctx.broadcast_tx {
        let mut line = event.to_json_line();
        line.push('\n');
        let _ = tx.send(line);
    } else {
        super::uds_cancel::emit_event(ctx.stdout, event).await;
    }
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

struct SetModelArgs {
    id: Option<String>,
    type_name: String,
    model: Option<String>,
    provider: Option<String>,
    model_id: Option<String>,
}

async fn handle_set_model(args: SetModelArgs, ctx: &mut DispatchCtx<'_>) -> bool {
    let SetModelArgs {
        id,
        type_name,
        model,
        provider,
        model_id,
    } = args;
    let resolved_model = match resolve_set_model_target(model, provider, model_id) {
        Ok(m) => m,
        Err(msg) => {
            let ev = AgentEvent::err(id.as_deref(), &type_name, msg);
            emit_event_to_broadcast_or_writer(ctx, &ev).await;
            return false;
        }
    };
    ctx.agent.set_model(resolved_model.clone());
    ctx.session.set_model(resolved_model);
    tracing::debug!(new_model = %ctx.session.model(), "UDS: model switched");
    let ev = AgentEvent::ok(id.as_deref(), &type_name, None);
    emit_event_to_broadcast_or_writer(ctx, &ev).await;
    false
}

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
        AgentCommand::GetExtensions { .. } => {
            Some(serde_json::json!({ "extensions": build_extension_list(ctx) }))
        }
        AgentCommand::ReloadExtensions { .. } => {
            // Handled in dispatch_command (needs async I/O + broadcast).
            None
        }
        _ => None,
    }
}

/// Dispatch a single UDS command.  Returns `true` if the loop should exit.
pub(super) async fn dispatch_command(cmd: AgentCommand, ctx: &mut DispatchCtx<'_>) -> bool {
    let id = cmd.id().map(str::to_owned);
    let type_name = cmd.type_name().to_owned();

    if let Some(data) = query_response_data(&cmd, ctx) {
        let ev = AgentEvent::ok(id.as_deref(), &type_name, Some(data));
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
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
        AgentCommand::Steer { message, .. } => {
            handle_steer(ctx, id.as_deref(), &type_name, message).await
        }
        AgentCommand::FollowUp { message, .. } => {
            handle_follow_up(ctx, id.as_deref(), &type_name, message).await
        }
        AgentCommand::Abort { .. } => handle_abort(ctx, id.as_deref(), &type_name).await,
        AgentCommand::SetModel {
            model,
            provider,
            model_id,
            ..
        } => {
            handle_set_model(
                SetModelArgs {
                    id,
                    type_name,
                    model,
                    provider,
                    model_id,
                },
                ctx,
            )
            .await
        }
        AgentCommand::ReloadExtensions { .. } => {
            handle_reload_extensions(ctx, id.as_deref(), &type_name).await;
            false
        }
        AgentCommand::GetExtensions { .. }
        | AgentCommand::GetState { .. }
        | AgentCommand::GetMessages { .. }
        | AgentCommand::GetMessagesTail { .. }
        | AgentCommand::GetSessionStats { .. } => {
            tracing::error!(command = %type_name, "query variant reached dispatch fallback");
            let ev = AgentEvent::err(
                id.as_deref(),
                &type_name,
                "internal: unhandled query command",
            );
            emit_event_to_broadcast_or_writer(ctx, &ev).await;
            false
        }
    }
}

async fn handle_steer(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    type_name: &str,
    message: String,
) -> bool {
    if ctx.session.is_streaming() {
        fire_cancel(&ctx.cancel_handle);
        ctx.session.prepend_pending(message);
    } else {
        ctx.session.enqueue_pending(message);
    }
    let ev = AgentEvent::ok(id, type_name, None);
    emit_event_to_broadcast_or_writer(ctx, &ev).await;
    false
}

async fn handle_follow_up(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    type_name: &str,
    message: String,
) -> bool {
    ctx.session.enqueue_pending(message);
    let ev = AgentEvent::ok(id, type_name, None);
    emit_event_to_broadcast_or_writer(ctx, &ev).await;
    false
}

async fn handle_abort(ctx: &mut DispatchCtx<'_>, id: Option<&str>, type_name: &str) -> bool {
    fire_cancel(&ctx.cancel_handle);
    let ev = AgentEvent::ok(id, type_name, None);
    emit_event_to_broadcast_or_writer(ctx, &ev).await;
    false
}

use super::uds_extensions::{build_extension_list, handle_reload_extensions};

struct PromptCommand {
    id: Option<String>,
    type_name: String,
    message: String,
    streaming_behavior: Option<StreamingBehavior>,
}

async fn handle_prompt(ctx: &mut DispatchCtx<'_>, cmd: PromptCommand) -> bool {
    let PromptCommand {
        id,
        type_name,
        message,
        streaming_behavior,
    } = cmd;

    if ctx.session.is_streaming() {
        match streaming_behavior {
            Some(StreamingBehavior::FollowUp) | Some(StreamingBehavior::Steer) => {
                ctx.session.enqueue_pending(message);
                let ev = AgentEvent::ok(id.as_deref(), &type_name, None);
                emit_event_to_broadcast_or_writer(ctx, &ev).await;
                return false;
            }
            None => {
                let ev = AgentEvent::err(
                    id.as_deref(),
                    &type_name,
                    "agent is running; provide streamingBehavior",
                );
                emit_event_to_broadcast_or_writer(ctx, &ev).await;
                return false;
            }
        }
    }

    let Some(cancel_rx) = arm_cancel(&ctx.cancel_handle) else {
        return false;
    };

    let outcome = if let Some(ref tx) = ctx.broadcast_tx {
        run_agent_prompt_broadcast(PromptArgsBroadcast {
            agent: ctx.agent,
            messages: ctx.messages,
            session: ctx.session,
            broadcast_tx: tx.clone(),
            message,
            cancel_rx,
        })
        .await
    } else {
        run_agent_prompt(PromptArgs {
            agent: ctx.agent,
            messages: ctx.messages,
            session: ctx.session,
            stdout: ctx.stdout,
            message,
            cancel_rx,
        })
        .await
    };

    disarm_cancel(&ctx.cancel_handle);

    match outcome {
        PromptOutcome::Cancelled => {
            drain_and_run_pending(ctx).await;
            false
        }
        PromptOutcome::Error => {
            // Error was already emitted.  Drain pending follow-ups so
            // they don't fire unexpectedly on a later prompt.
            drain_and_run_pending(ctx).await;
            false
        }
        PromptOutcome::Success => {
            let ev = AgentEvent::ok(id.as_deref(), &type_name, None);
            emit_event_to_broadcast_or_writer(ctx, &ev).await;
            drain_and_run_pending(ctx).await;
            false
        }
    }
}

async fn drain_and_run_pending(ctx: &mut DispatchCtx<'_>) {
    loop {
        let pending = ctx.session.drain_pending();
        if pending.is_empty() {
            break;
        }
        for follow_msg in pending {
            let Some(cancel_rx) = arm_cancel(&ctx.cancel_handle) else {
                break;
            };
            if let Some(ref tx) = ctx.broadcast_tx {
                run_agent_prompt_broadcast(PromptArgsBroadcast {
                    agent: ctx.agent,
                    messages: ctx.messages,
                    session: ctx.session,
                    broadcast_tx: tx.clone(),
                    message: follow_msg,
                    cancel_rx,
                })
                .await;
            } else {
                run_agent_prompt(PromptArgs {
                    agent: ctx.agent,
                    messages: ctx.messages,
                    session: ctx.session,
                    stdout: ctx.stdout,
                    message: follow_msg,
                    cancel_rx,
                })
                .await;
            }
            disarm_cancel(&ctx.cancel_handle);
        }
    }
}

#[cfg(test)]
#[path = "uds_tests.rs"]
mod tests;
