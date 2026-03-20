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

/// Atomic counter for assigning unique client IDs (#352).
static NEXT_CLIENT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

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
    /// When true, keep the agent alive after all clients disconnect (#348).
    pub persist: bool,
    /// Receiver for subagent notifications (#523).
    pub notification_rx: Option<crate::infrastructure::tools::subagent_registry::NotificationRx>,
    /// Shared subagent registry for get_subagents / state_changed (#524).
    pub subagent_registry:
        Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
}

/// A command line from a client.
struct ClientCommand {
    line: String,
    /// Unique client identifier for per-client tool routing (#352).
    client_id: u64,
}

/// Sentinel: a client disconnected.
struct ClientDisconnected {
    /// Which client disconnected (#352).
    client_id: u64,
}

/// Messages from client reader tasks to the dispatch loop.
enum ClientMessage {
    Command(ClientCommand),
    Disconnected(ClientDisconnected),
}

/// RAII guard that decrements `live_clients` on drop (normal exit or panic).
struct ClientGuard {
    live_clients: std::sync::Arc<std::sync::atomic::AtomicU32>,
    cmd_tx: tokio::sync::mpsc::Sender<ClientMessage>,
    /// Unique client identifier for per-client tool tracking (#352).
    client_id: u64,
}

impl Drop for ClientGuard {
    fn drop(&mut self) {
        self.live_clients
            .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        // Best-effort sentinel — if channel is closed the dispatch loop already
        // exited, so the message is not needed.
        if let Err(e) = self
            .cmd_tx
            .try_send(ClientMessage::Disconnected(ClientDisconnected {
                client_id: self.client_id,
            }))
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
    let persist = args.persist;
    let notification_rx = args.notification_rx;
    let subagent_registry = args.subagent_registry;
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

    // Drop our clone so cmd_rx closes when all client senders (accept loop)
    // are gone.  The accept loop's clone keeps the channel open while
    // it runs — the `!persist` guard in `run_dispatch_loop` controls shutdown.
    drop(cmd_tx);

    let mut null_writer: Box<dyn tokio::io::AsyncWrite + Send + Unpin> =
        Box::new(tokio::io::sink());
    let client_tool_registry = super::uds_ext_protocol::new_client_tool_registry();
    let mut ctx = DispatchCtx {
        agent: &mut agent,
        messages: &mut messages,
        session: &mut agent_session,
        stdout: &mut *null_writer,
        session_key: &session_key,
        cancel_handle,
        broadcast_tx: Some(broadcast_tx),
        ext_registry,
        client_tool_registry: client_tool_registry.clone(),
        current_client_id: 0,
        subagent_registry,
        notification_rx,
    };

    run_dispatch_loop(
        &mut ctx,
        DispatchLoopArgs { cmd_rx, persist },
        &live_clients,
    )
    .await;

    accept_task.abort();

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
                    let client_id =
                        NEXT_CLIENT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let guard = ClientGuard {
                        live_clients: live_clients.clone(),
                        cmd_tx: cmd_tx.clone(),
                        client_id,
                    };
                    let args = ClientHandlerArgs {
                        stream,
                        broadcast_rx: broadcast_tx.subscribe(),
                        cmd_tx: cmd_tx.clone(),
                        cancel_handle: cancel_handle.clone(),
                        client_id,
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

/// Arguments for [`run_dispatch_loop`].
struct DispatchLoopArgs {
    cmd_rx: tokio::sync::mpsc::Receiver<ClientMessage>,
    persist: bool,
}

/// Process commands from all clients until no clients remain or a fatal error.
/// Also drains subagent notifications and injects them as follow-up messages (#523).
/// During prompt execution, notifications are drained by run_with_token_drain_broadcast (#534).
async fn run_dispatch_loop(
    ctx: &mut DispatchCtx<'_>,
    args: DispatchLoopArgs,
    live_clients: &std::sync::atomic::AtomicU32,
) {
    let DispatchLoopArgs {
        mut cmd_rx,
        persist,
    } = args;
    loop {
        let msg = recv_next_message(&mut cmd_rx, &mut ctx.notification_rx).await;
        let Some(msg) = msg else { break };
        match msg {
            DispatchMsg::Client(client_msg) => {
                if handle_client_msg(ctx, client_msg, persist, live_clients).await {
                    break;
                }
            }
            DispatchMsg::Notification(notif) => {
                let message = notif.to_message();
                tracing::info!(msg = %message, "injecting subagent notification");
                ctx.session.enqueue_pending(message);
                // Broadcast state_changed event to all UDS clients (#524).
                let list = super::protocol::build_subagent_info_list(&ctx.subagent_registry);
                let ev = AgentEvent::SubagentStateChanged { subagents: list };
                emit_event_to_broadcast_or_writer(ctx, &ev).await;
            }
        }
    }
}

enum DispatchMsg {
    Client(ClientMessage),
    Notification(crate::infrastructure::tools::subagent_registry::SubagentNotification),
}

async fn recv_next_message(
    cmd_rx: &mut tokio::sync::mpsc::Receiver<ClientMessage>,
    notification_rx: &mut Option<crate::infrastructure::tools::subagent_registry::NotificationRx>,
) -> Option<DispatchMsg> {
    if let Some(rx) = notification_rx {
        tokio::select! {
            biased;
            client_msg = cmd_rx.recv() => client_msg.map(DispatchMsg::Client),
            Some(notif) = rx.recv() => Some(DispatchMsg::Notification(notif)),
        }
    } else {
        cmd_rx.recv().await.map(DispatchMsg::Client)
    }
}

/// Handle a single client message. Returns `true` if the loop should exit.
async fn handle_client_msg(
    ctx: &mut DispatchCtx<'_>,
    client_msg: ClientMessage,
    persist: bool,
    live_clients: &std::sync::atomic::AtomicU32,
) -> bool {
    match client_msg {
        ClientMessage::Command(cmd) => {
            ctx.current_client_id = cmd.client_id;
            match parse_line(&cmd.line) {
                LineResult::LineTooLong => {
                    let ev = AgentEvent::err(None, "parse_error", "line exceeds 1 MiB limit");
                    emit_event_to_broadcast_or_writer(ctx, &ev).await;
                }
                LineResult::ParseError(e) if e.is_empty() => {}
                LineResult::ParseError(_) => {
                    let ev = AgentEvent::Response {
                        id: None,
                        command: "parse_error".to_string(),
                        success: false,
                        data: None,
                        error: Some("invalid JSON command".to_string()),
                    };
                    emit_event_to_broadcast_or_writer(ctx, &ev).await;
                }
                LineResult::Command(parsed) => {
                    if dispatch_command(parsed, ctx).await {
                        return true;
                    }
                }
            }
            false
        }
        ClientMessage::Disconnected(disc) => {
            handle_disconnect(ctx, disc.client_id).await;
            !persist && live_clients.load(std::sync::atomic::Ordering::SeqCst) == 0
        }
    }
}

/// Unregister tools owned by a disconnecting client (#352).
async fn handle_disconnect(ctx: &mut DispatchCtx<'_>, client_id: u64) {
    let removed =
        super::uds_ext_protocol::handle_client_disconnect(client_id, &ctx.client_tool_registry);
    if !removed.is_empty() {
        for name in &removed {
            ctx.agent.unregister_extension_tool(name);
        }
        let ext_names = ctx.agent.tool_registry_extension_names();
        let changed =
            super::uds_ext_protocol::build_extensions_changed_event(&ext_names, ctx.agent);
        emit_event_to_broadcast_or_writer(ctx, &changed).await;
    }
}

// ─── Per-client handler ───────────────────────────────────────────────────────

/// Arguments for [`handle_client`].
struct ClientHandlerArgs {
    stream: tokio::net::UnixStream,
    broadcast_rx: tokio::sync::broadcast::Receiver<String>,
    cmd_tx: tokio::sync::mpsc::Sender<ClientMessage>,
    cancel_handle: CancelHandle,
    /// Unique client identifier (#352).
    client_id: u64,
    /// RAII guard — decrements `live_clients` and sends `Disconnected` on drop.
    _guard: ClientGuard,
}

async fn handle_client(args: ClientHandlerArgs) {
    let ClientHandlerArgs {
        stream,
        mut broadcast_rx,
        cmd_tx,
        cancel_handle,
        client_id,
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
        let msg = ClientMessage::Command(ClientCommand { line, client_id });
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
    /// Subagent notification receiver — drained during prompt execution (#534).
    pub notification_rx:
        &'a mut Option<crate::infrastructure::tools::subagent_registry::NotificationRx>,
    /// Subagent registry for building state-changed events (#534).
    pub subagent_registry:
        &'a Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
}

/// Run an agent prompt, emitting events to the broadcast channel.
pub(super) async fn run_agent_prompt_broadcast(args: PromptArgsBroadcast<'_>) -> PromptOutcome {
    use crate::domain::agent::AgentProgressEvent;
    use crate::interface::cli::protocol::{TurnMessage, TurnUsage};

    let PromptArgsBroadcast {
        agent,
        messages,
        session,
        broadcast_tx,
        message,
        cancel_rx,
        notification_rx,
        subagent_registry,
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

    let drain_result = run_with_token_drain_broadcast(TokenDrainBroadcastArgs {
        broadcast_tx: &broadcast_tx,
        agent,
        messages,
        progress_rx: &mut progress_rx,
        cancel_rx,
        notification_rx,
        subagent_registry,
    })
    .await;

    agent.set_progress_callback(None);
    session.set_streaming(false);

    // Enqueue notification messages collected during prompt execution (#534).
    // These will be processed as follow-up prompts by drain_and_run_pending.
    for msg in drain_result.notification_messages {
        session.enqueue_pending(msg);
    }

    match drain_result.result {
        None => {
            messages.truncate(user_msg_idx);
            PromptOutcome::Cancelled
        }
        Some(Ok(agent_result)) => {
            // Post-hoc tool events are NOT emitted here — ToolStarted/Finished
            // are already forwarded in real-time via forward_progress_event_broadcast.
            // Emitting them again would cause duplicate events with conflicting IDs.
            let total = agent_result
                .input_tokens
                .saturating_add(agent_result.output_tokens);
            let usage = if total > 0 {
                Some(TurnUsage {
                    input: agent_result.input_tokens,
                    output: agent_result.output_tokens,
                    total,
                })
            } else {
                None
            };
            let turn_end = AgentEvent::TurnEnd {
                message: TurnMessage {
                    role: "assistant".to_string(),
                    content: agent_result.response.clone(),
                    usage,
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
    /// Subagent notification receiver — drained alongside tokens (#534).
    notification_rx:
        &'a mut Option<crate::infrastructure::tools::subagent_registry::NotificationRx>,
    /// Subagent registry for building state-changed events (#534).
    subagent_registry:
        &'a Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
}

/// Result of run_with_token_drain_broadcast, including collected notification messages (#534).
struct TokenDrainResult {
    result: Option<Result<crate::domain::agent::AgentResult, crate::domain::error::DomainError>>,
    /// Notification messages collected during prompt execution, to be enqueued as pending.
    notification_messages: Vec<String>,
}

async fn run_with_token_drain_broadcast(args: TokenDrainBroadcastArgs<'_>) -> TokenDrainResult {
    use crate::domain::agent::AgentLoop;

    let TokenDrainBroadcastArgs {
        broadcast_tx,
        agent,
        messages,
        progress_rx,
        cancel_rx,
        notification_rx,
        subagent_registry,
    } = args;

    tokio::pin!(cancel_rx);
    let mut process_fut = agent.process(messages);
    let mut notification_messages = Vec::new();

    let result = loop {
        // Build a future that drains notification_rx if present (#534).
        // This ensures SubagentStateChanged events propagate to TUI clients
        // in real-time during prompt execution, not just between prompts.
        let notif_recv = async {
            if let Some(rx) = notification_rx.as_mut() {
                rx.recv().await
            } else {
                std::future::pending().await
            }
        };

        tokio::select! {
            biased;
            _ = &mut cancel_rx => break None,
            Some(ev) = progress_rx.recv() => {
                forward_progress_event_broadcast(ev, broadcast_tx);
            }
            Some(notif) = notif_recv => {
                // Broadcast state-changed event to TUI AND collect message for LLM injection.
                notification_messages.push(notif.to_message());
                forward_notification_broadcast(notif, broadcast_tx, subagent_registry);
            }
            result = &mut process_fut => {
                while let Ok(ev) = progress_rx.try_recv() {
                    forward_progress_event_broadcast(ev, broadcast_tx);
                }
                if let Some(rx) = notification_rx.as_mut() {
                    while let Ok(notif) = rx.try_recv() {
                        notification_messages.push(notif.to_message());
                        forward_notification_broadcast(notif, broadcast_tx, subagent_registry);
                    }
                }
                break Some(result);
            }
        }
    };

    TokenDrainResult {
        result,
        notification_messages,
    }
}

/// Forward a subagent notification as a SubagentStateChanged broadcast event (#534).
fn forward_notification_broadcast(
    notif: crate::infrastructure::tools::subagent_registry::SubagentNotification,
    broadcast_tx: &tokio::sync::broadcast::Sender<String>,
    subagent_registry: &Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
) {
    let message = notif.to_message();
    tracing::info!(msg = %message, "injecting subagent notification during prompt");
    // Build full subagent info list from registry for the state-changed event.
    let list = super::protocol::build_subagent_info_list(subagent_registry);
    let ev = AgentEvent::SubagentStateChanged { subagents: list };
    broadcast_event(broadcast_tx, &ev);
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
            result_content,
            is_error,
            ..
        } => {
            broadcast_event(
                tx,
                &AgentEvent::ToolExecutionEnd {
                    tool_call_id,
                    tool_name: name,
                    result: crate::interface::cli::protocol::ToolResultContent {
                        content: vec![serde_json::json!({"type":"text","text": result_content})],
                    },
                    is_error,
                },
            );
        }
        _ => {}
    }
}
