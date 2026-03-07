//! Multi-client UDS event bus (#318).
//!
//! Accept loop + broadcast: multiple clients connect to the same socket.
//! Events are serialized once and delivered to all clients via
//! `tokio::sync::broadcast`.  Commands from all clients merge into a
//! shared `mpsc` channel → single dispatch loop (no concurrent session
//! mutation).  Agent shuts down when all clients disconnect.

use crate::application::agent_loop::AgentLoopImpl;
use crate::domain::message::Message;
use crate::domain::session::{Session, SessionStore};

use super::protocol::AgentEvent;
use super::uds::{
    DispatchCtx, LineResult, MAX_LINE_BYTES, dispatch_command, emit_event_to_broadcast_or_writer,
    inject_system_prompt, is_cancel_command, parse_line, remove_injected_system_prompt,
};
use super::uds_cancel::{CancelHandle, CancelSlot, PromptOutcome, fire_cancel};
use super::uds_session::{AgentSession, message_to_json};

// ─── Constants ────────────────────────────────────────────────────────────────

/// Maximum number of concurrent client connections.
const MAX_CLIENTS: u32 = 64;

// ─── Types ────────────────────────────────────────────────────────────────────

pub(super) struct MultiClientArgs {
    pub agent: AgentLoopImpl,
    pub messages: Vec<Message>,
    pub model: String,
    pub session_key: String,
    pub ephemeral: bool,
    pub system_prompt: String,
    /// Shared extension registry for get_extensions / reload_extensions.
    pub ext_registry: Option<
        std::sync::Arc<
            std::sync::Mutex<crate::infrastructure::extensions::registry::ExtensionRegistry>,
        >,
    >,
    /// When `Some`, spawn a hot-reload watcher at the given poll interval.
    pub hot_reload_interval: Option<std::time::Duration>,
}

/// A command line from a client.
struct ClientCommand {
    line: String,
}

/// Sentinel: a client disconnected.
struct ClientDisconnected;

/// Messages from client reader tasks to the dispatch loop.
enum ClientMessage {
    Command(ClientCommand),
    Disconnected(ClientDisconnected),
}

/// RAII guard that decrements `live_clients` on drop (normal exit or panic).
struct ClientGuard {
    live_clients: std::sync::Arc<std::sync::atomic::AtomicU32>,
    cmd_tx: tokio::sync::mpsc::Sender<ClientMessage>,
}

impl Drop for ClientGuard {
    fn drop(&mut self) {
        self.live_clients
            .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        // Best-effort sentinel — if channel is closed the dispatch loop already
        // exited, so the message is not needed.
        if let Err(e) = self
            .cmd_tx
            .try_send(ClientMessage::Disconnected(ClientDisconnected))
        {
            tracing::debug!("disconnect sentinel not delivered: {e}");
        }
    }
}

// ─── Accept loop + dispatch ───────────────────────────────────────────────────

pub(super) async fn multi_client_loop(
    args: MultiClientArgs,
    listener: tokio::net::UnixListener,
    session_store: &dyn SessionStore,
) -> i32 {
    let ext_registry = args.ext_registry;
    let hot_reload_interval = args.hot_reload_interval;
    let MultiClientArgs {
        mut agent,
        mut messages,
        model,
        session_key,
        ephemeral,
        system_prompt,
        ..
    } = args;

    inject_system_prompt(&mut messages, &system_prompt);

    let mut agent_session = AgentSession::new(model, session_key.clone());

    let (broadcast_tx, _) = tokio::sync::broadcast::channel::<String>(256);
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel::<ClientMessage>(256);
    let cancel_handle: CancelHandle = std::sync::Arc::new(std::sync::Mutex::new(CancelSlot::Idle));
    let live_clients = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));

    let accept_task = spawn_accept_loop(AcceptLoopArgs {
        listener,
        broadcast_tx: broadcast_tx.clone(),
        cmd_tx: cmd_tx.clone(),
        cancel_handle: cancel_handle.clone(),
        live_clients: live_clients.clone(),
    });

    // Spawn hot-reload watcher if an extension registry and interval are configured.
    // The callback sends a `reload_extensions` command through `cmd_tx` so the
    // dispatch loop (which has `&mut AgentLoopImpl`) can sync the tool registry.
    let watcher_task = if let (Some(ext_reg), Some(interval)) = (&ext_registry, hot_reload_interval)
    {
        let watcher_ext_reg = ext_reg.clone();
        let watcher_cmd_tx = cmd_tx.clone();
        Some(
            crate::infrastructure::extensions::watcher::spawn_watcher_with_callback(
                watcher_ext_reg,
                interval,
                std::sync::Arc::new(move |_count| {
                    let cmd = ClientMessage::Command(ClientCommand {
                        line: r#"{"type":"reload_extensions","id":"hot-reload"}"#.to_string(),
                    });
                    // Use try_send (not blocking_send) since this runs inside
                    // an async context (tokio::spawn). Channel buffer is large
                    // enough that this should not fail under normal load.
                    let _ = watcher_cmd_tx.try_send(cmd);
                }),
            ),
        )
    } else {
        None
    };

    // Drop our clone so cmd_rx closes when all clients are gone.
    drop(cmd_tx);

    let mut null_writer: Box<dyn tokio::io::AsyncWrite + Send + Unpin> =
        Box::new(tokio::io::sink());
    let mut ctx = DispatchCtx {
        agent: &mut agent,
        messages: &mut messages,
        session: &mut agent_session,
        stdout: &mut *null_writer,
        session_key: &session_key,
        cancel_handle,
        broadcast_tx: Some(broadcast_tx),
        ext_registry,
    };

    run_dispatch_loop(&mut ctx, cmd_rx, &live_clients).await;

    accept_task.abort();
    if let Some(watcher) = watcher_task {
        watcher.abort();
    }

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

/// Arguments for [`spawn_accept_loop`].
struct AcceptLoopArgs {
    listener: tokio::net::UnixListener,
    broadcast_tx: tokio::sync::broadcast::Sender<String>,
    cmd_tx: tokio::sync::mpsc::Sender<ClientMessage>,
    cancel_handle: CancelHandle,
    live_clients: std::sync::Arc<std::sync::atomic::AtomicU32>,
}

/// Spawn the accept loop that listens for new client connections.
fn spawn_accept_loop(args: AcceptLoopArgs) -> tokio::task::JoinHandle<()> {
    let AcceptLoopArgs {
        listener,
        broadcast_tx,
        cmd_tx,
        cancel_handle,
        live_clients,
    } = args;
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => {
                    let current = live_clients.load(std::sync::atomic::Ordering::SeqCst);
                    if current >= MAX_CLIENTS {
                        tracing::warn!(
                            current,
                            max = MAX_CLIENTS,
                            "rejecting connection: max clients reached"
                        );
                        drop(stream);
                        continue;
                    }
                    live_clients.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let guard = ClientGuard {
                        live_clients: live_clients.clone(),
                        cmd_tx: cmd_tx.clone(),
                    };
                    let args = ClientHandlerArgs {
                        stream,
                        broadcast_rx: broadcast_tx.subscribe(),
                        cmd_tx: cmd_tx.clone(),
                        cancel_handle: cancel_handle.clone(),
                        _guard: guard,
                    };
                    tokio::spawn(async move { handle_client(args).await });
                }
                Err(e) => {
                    tracing::warn!("accept error: {e}");
                    break;
                }
            }
        }
    })
}

/// Process commands from all clients until no clients remain or a fatal error.
async fn run_dispatch_loop(
    ctx: &mut DispatchCtx<'_>,
    mut cmd_rx: tokio::sync::mpsc::Receiver<ClientMessage>,
    live_clients: &std::sync::atomic::AtomicU32,
) {
    while let Some(client_msg) = cmd_rx.recv().await {
        match client_msg {
            ClientMessage::Command(cmd) => match parse_line(&cmd.line) {
                LineResult::LineTooLong => {
                    let ev = AgentEvent::err(None, "parse_error", "line exceeds 1 MiB limit");
                    emit_event_to_broadcast_or_writer(ctx, &ev).await;
                }
                LineResult::ParseError(e) if e.is_empty() => {}
                LineResult::ParseError(_) => {
                    // Suppress raw error content — broadcasting the parse error
                    // from one client would leak its command content to all
                    // connected clients.
                    let ev = AgentEvent::Response {
                        id: None,
                        command: "parse_error".to_string(),
                        success: false,
                        data: None,
                        error: Some("invalid JSON command".to_string()),
                    };
                    emit_event_to_broadcast_or_writer(ctx, &ev).await;
                }
                LineResult::Command(cmd) => {
                    if dispatch_command(cmd, ctx).await {
                        break;
                    }
                }
            },
            ClientMessage::Disconnected(_) => {
                if live_clients.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                    break;
                }
            }
        }
    }
}

// ─── Per-client handler ───────────────────────────────────────────────────────

/// Arguments for [`handle_client`].
struct ClientHandlerArgs {
    stream: tokio::net::UnixStream,
    broadcast_rx: tokio::sync::broadcast::Receiver<String>,
    cmd_tx: tokio::sync::mpsc::Sender<ClientMessage>,
    cancel_handle: CancelHandle,
    /// RAII guard — decrements `live_clients` and sends `Disconnected` on drop.
    _guard: ClientGuard,
}

async fn handle_client(args: ClientHandlerArgs) {
    let ClientHandlerArgs {
        stream,
        mut broadcast_rx,
        cmd_tx,
        cancel_handle,
        _guard,
    } = args;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let (reader, mut writer) = tokio::io::split(stream);
    let mut lines = BufReader::new(reader).lines();

    // Writer task: drain broadcast → this client.
    let writer_task = tokio::spawn(async move {
        loop {
            match broadcast_rx.recv().await {
                Ok(line) => {
                    if writer.write_all(line.as_bytes()).await.is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("client lagged by {n} events");
                    // Notify the client so it can request a state refresh.
                    let msg = format!(
                        "{{\"type\":\"error\",\"message\":\"dropped {} events — use get_messages to re-sync\"}}\n",
                        n
                    );
                    if writer.write_all(msg.as_bytes()).await.is_err() {
                        break;
                    }
                    continue;
                }
            }
        }
    });

    // Reader loop: commands → dispatch mpsc.
    // Guard oversized lines — `next_line()` allocates the full line before
    // returning, but we drop it immediately if it exceeds MAX_LINE_BYTES
    // to prevent queuing oversized payloads in the mpsc channel.
    while let Ok(Some(line)) = lines.next_line().await {
        if line.len() > MAX_LINE_BYTES {
            // Already allocated unfortunately (AsyncBufReadExt limitation),
            // but at least we don't queue it in the mpsc channel.
            tracing::warn!(len = line.len(), "dropping oversized line from client");
            continue;
        }
        if is_cancel_command(line.trim()) {
            fire_cancel(&cancel_handle);
        }
        let msg = ClientMessage::Command(ClientCommand { line });
        if cmd_tx.send(msg).await.is_err() {
            break;
        }
    }

    writer_task.abort();
    // `_guard` is dropped here (or on panic), which decrements live_clients
    // and sends the Disconnected sentinel.
}

// ─── Broadcast prompt execution ───────────────────────────────────────────────

/// Arguments for [`run_agent_prompt_broadcast`].
pub(super) struct PromptArgsBroadcast<'a> {
    pub agent: &'a mut AgentLoopImpl,
    pub messages: &'a mut Vec<Message>,
    pub session: &'a mut AgentSession,
    pub broadcast_tx: tokio::sync::broadcast::Sender<String>,
    pub message: String,
    pub cancel_rx: tokio::sync::oneshot::Receiver<()>,
}

/// Run an agent prompt, emitting events to the broadcast channel.
pub(super) async fn run_agent_prompt_broadcast(args: PromptArgsBroadcast<'_>) -> PromptOutcome {
    use crate::domain::agent::AgentProgressEvent;
    use crate::interface::cli::protocol::TurnMessage;

    let PromptArgsBroadcast {
        agent,
        messages,
        session,
        broadcast_tx,
        message,
        cancel_rx,
    } = args;

    session.set_streaming(true);
    broadcast_event(&broadcast_tx, &AgentEvent::AgentStart);
    broadcast_event(&broadcast_tx, &AgentEvent::TurnStart);

    let user_msg_idx = messages.len();
    messages.push(Message::user(message));
    let before_len = messages.len();

    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel::<AgentProgressEvent>(256);
    agent.set_progress_callback(Some(std::sync::Arc::new(move |ev| {
        let _ = progress_tx.try_send(ev);
    })));

    let result = run_with_token_drain_broadcast(TokenDrainBroadcastArgs {
        broadcast_tx: &broadcast_tx,
        agent,
        messages,
        progress_rx: &mut progress_rx,
        cancel_rx,
    })
    .await;

    agent.set_progress_callback(None);
    session.set_streaming(false);

    match result {
        None => {
            messages.truncate(user_msg_idx);
            PromptOutcome::Cancelled
        }
        Some(Ok(agent_result)) => {
            // Post-hoc tool events are NOT emitted here — ToolStarted/Finished
            // are already forwarded in real-time via forward_progress_event_broadcast.
            // Emitting them again would cause duplicate events with conflicting IDs.
            let turn_end = AgentEvent::TurnEnd {
                message: TurnMessage {
                    role: "assistant".to_string(),
                    content: agent_result.response.clone(),
                    usage: None,
                    stop_reason: None,
                },
                tool_results: vec![],
            };
            broadcast_event(&broadcast_tx, &turn_end);
            let run_msgs: Vec<serde_json::Value> =
                messages[before_len..].iter().map(message_to_json).collect();
            broadcast_event(&broadcast_tx, &AgentEvent::AgentEnd { messages: run_msgs });
            PromptOutcome::Success
        }
        Some(Err(e)) => {
            broadcast_event(
                &broadcast_tx,
                &AgentEvent::err(None, "agent_error", format!("{e}")),
            );
            PromptOutcome::Error
        }
    }
}

fn broadcast_event(tx: &tokio::sync::broadcast::Sender<String>, event: &AgentEvent) {
    let mut line = event.to_json_line();
    line.push('\n');
    let _ = tx.send(line);
}

/// Arguments for [`run_with_token_drain_broadcast`].
struct TokenDrainBroadcastArgs<'a> {
    broadcast_tx: &'a tokio::sync::broadcast::Sender<String>,
    agent: &'a mut AgentLoopImpl,
    messages: &'a mut Vec<Message>,
    progress_rx: &'a mut tokio::sync::mpsc::Receiver<crate::domain::agent::AgentProgressEvent>,
    cancel_rx: tokio::sync::oneshot::Receiver<()>,
}

async fn run_with_token_drain_broadcast(
    args: TokenDrainBroadcastArgs<'_>,
) -> Option<Result<crate::domain::agent::AgentResult, crate::domain::error::DomainError>> {
    use crate::domain::agent::AgentLoop;

    let TokenDrainBroadcastArgs {
        broadcast_tx,
        agent,
        messages,
        progress_rx,
        cancel_rx,
    } = args;

    tokio::pin!(cancel_rx);
    let mut process_fut = agent.process(messages);

    loop {
        tokio::select! {
            biased;
            _ = &mut cancel_rx => return None,
            Some(ev) = progress_rx.recv() => {
                forward_progress_event_broadcast(ev, broadcast_tx);
            }
            result = &mut process_fut => {
                while let Ok(ev) = progress_rx.try_recv() {
                    forward_progress_event_broadcast(ev, broadcast_tx);
                }
                return Some(result);
            }
        }
    }
}

fn forward_progress_event_broadcast(
    ev: crate::domain::agent::AgentProgressEvent,
    tx: &tokio::sync::broadcast::Sender<String>,
) {
    use crate::domain::agent::AgentProgressEvent;
    match ev {
        AgentProgressEvent::Token(t) => {
            broadcast_event(tx, &AgentEvent::Token { token: t });
        }
        AgentProgressEvent::ToolStarted {
            tool_call_id,
            name,
            arguments,
        } => {
            let args: serde_json::Value = serde_json::from_str(&arguments).unwrap_or_default();
            broadcast_event(
                tx,
                &AgentEvent::ToolExecutionStart {
                    tool_call_id,
                    tool_name: name,
                    args,
                },
            );
        }
        AgentProgressEvent::ToolFinished {
            tool_call_id,
            name,
            is_error,
            ..
        } => {
            broadcast_event(
                tx,
                &AgentEvent::ToolExecutionEnd {
                    tool_call_id,
                    tool_name: name,
                    result: crate::interface::cli::protocol::ToolResultContent {
                        content: vec![serde_json::json!({"type":"text","text":""})],
                    },
                    is_error,
                },
            );
        }
        _ => {}
    }
}
