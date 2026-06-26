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

/// Broadcast channel capacity for UDS event delivery.
/// Shared between the early-creation path (workflow) and the default path.
pub(super) const BROADCAST_CHANNEL_CAPACITY: usize = 256;

/// Atomic counter for assigning unique client IDs (#352).
static NEXT_CLIENT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

// ─── Types ────────────────────────────────────────────────────────────────────

pub(super) struct MultiClientArgs<'a> {
    pub agent: AgentLoopImpl,
    pub base_dir: &'a std::path::Path,
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
    /// Shared workflow state for auto-nudge injection (#562).
    pub workflow_state: Option<crate::interface::shared::WorkflowStateHandle>,
    /// Workflow config (auto_continue, completion_nudge flags).
    pub workflow_config: Option<crate::domain::workflow::WorkflowConfig>,
    /// Pre-created broadcast channel for workflow event emission (#598).
    pub broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    pub provider_reload: Option<&'a mut super::provider_reload::ProviderReload>,
    pub provider_reload_inputs: Option<&'a super::provider_reload::ProviderReloadInputs>,
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
    args: MultiClientArgs<'_>,
    listener: tokio::net::UnixListener,
    session_store: &dyn SessionStore,
) -> i32 {
    let ext_registry = args.ext_registry;
    let persist = args.persist;
    let notification_rx = args.notification_rx;
    let subagent_registry = args.subagent_registry;
    let wf_state = args.workflow_state;
    let wf_config = args.workflow_config;
    let pre_broadcast_tx = args.broadcast_tx;
    let provider_reload = args.provider_reload;
    let provider_reload_inputs = args.provider_reload_inputs;
    let MultiClientArgs {
        mut agent,
        base_dir,
        mut messages,
        model,
        mut session_key,
        ephemeral,
        system_prompt,
        ..
    } = args;

    inject_system_prompt(&mut messages, &system_prompt);

    let mut agent_session = AgentSession::new(model, session_key.clone());

    // Use the pre-created broadcast channel when available (workflow emitter
    // is already wired to it), otherwise create a fresh one (#598).
    let broadcast_tx = pre_broadcast_tx
        .unwrap_or_else(|| tokio::sync::broadcast::channel::<String>(BROADCAST_CHANNEL_CAPACITY).0);
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel::<ClientMessage>(256);
    let cancel_handle: CancelHandle = std::sync::Arc::new(std::sync::Mutex::new(CancelSlot::Idle));
    let live_clients = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));

    let client_tool_registry = super::uds_ext_protocol::new_client_tool_registry();

    let accept_task = spawn_accept_loop(AcceptLoopArgs {
        listener,
        broadcast_tx: broadcast_tx.clone(),
        cmd_tx: cmd_tx.clone(),
        cancel_handle: cancel_handle.clone(),
        live_clients: live_clients.clone(),
        client_tool_registry: client_tool_registry.clone(),
    });

    // Drop our clone so cmd_rx closes when all client senders (accept loop)
    // are gone.  The accept loop's clone keeps the channel open while
    // it runs — the `!persist` guard in `run_dispatch_loop` controls shutdown.
    drop(cmd_tx);

    let mut null_writer: Box<dyn tokio::io::AsyncWrite + Send + Unpin> =
        Box::new(tokio::io::sink());
    let mut ctx = DispatchCtx {
        base_dir,
        agent: &mut agent,
        messages: &mut messages,
        session: &mut agent_session,
        stdout: &mut *null_writer,
        session_key: &mut session_key,
        session_store,
        ephemeral,
        system_prompt: &system_prompt,
        cancel_handle,
        broadcast_tx: Some(broadcast_tx),
        ext_registry,
        client_tool_registry: client_tool_registry.clone(),
        current_client_id: 0,
        subagent_registry,
        notification_rx,
        workflow_state: wf_state.clone(),
        workflow_config: wf_config,
        provider_reload,
        provider_reload_inputs,
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
            workflow_run: wf_state
                .as_ref()
                .and_then(|ws| ws.lock().ok().and_then(|engine| engine.persisted_run())),
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
    /// Shared per-client tool state. Cloned into each ClientHandlerArgs
    /// so the reader task can resolve `tool_result` commands inline
    /// without routing them through the single-threaded dispatch loop
    /// (which is blocked on the agent's in-flight prompt).
    client_tool_registry: super::uds_ext_protocol::ClientToolRegistry,
}

/// Spawn the accept loop that listens for new client connections.
fn spawn_accept_loop(args: AcceptLoopArgs) -> tokio::task::JoinHandle<()> {
    let AcceptLoopArgs {
        listener,
        broadcast_tx,
        cmd_tx,
        cancel_handle,
        live_clients,
        client_tool_registry,
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
                    // Per-client targeted event channel (V4): the
                    // forwarder task for tools this client registers
                    // will route `execute_tool` events here — NOT on
                    // the broadcast — so they don't leak across
                    // clients. 64-slot buffer: any single LLM turn
                    // issues only a handful of tool calls, so the
                    // common-case depth is 1–2; the channel fills up
                    // only if the client stops reading.
                    let (targeted_tx, targeted_rx) = tokio::sync::mpsc::channel::<String>(64);
                    super::uds_ext_protocol::register_client_writer(
                        &client_tool_registry,
                        client_id,
                        targeted_tx,
                    );

                    let args = ClientHandlerArgs {
                        stream,
                        broadcast_rx: broadcast_tx.subscribe(),
                        targeted_rx,
                        cmd_tx: cmd_tx.clone(),
                        cancel_handle: cancel_handle.clone(),
                        client_id,
                        client_tool_registry: client_tool_registry.clone(),
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
                let (agent_id, sequence) = notif.dedupe_key();
                tracing::info!(%agent_id, sequence, "recording subagent completion note");
                // Auto-await (#816): enqueue the one-line note for delivery at the
                // parent's NEXT idle boundary. `enqueue_subagent_notification`
                // records the dedupe sequence internally and returns whether this
                // completion is new — so we don't also call
                // `record_subagent_notification` (that would double-dedupe).
                let is_new = ctx.session.enqueue_subagent_notification(
                    agent_id.clone(),
                    sequence,
                    notif.to_message(),
                );
                if is_new {
                    let ev = AgentEvent::SubagentNotification {
                        agent_id,
                        sequence,
                        message: notif.to_message(),
                    };
                    emit_event_to_broadcast_or_writer(ctx, &ev).await;
                }
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
    Notification(crate::infrastructure::tools::subagent_registry::SequencedSubagentNotification),
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
    /// Per-client targeted event stream — receives events addressed
    /// to this client only (currently just `execute_tool` from
    /// forwarder tasks). Writer_task selects over this AND
    /// broadcast_rx so targeted events aren't visible to other
    /// clients.
    targeted_rx: tokio::sync::mpsc::Receiver<String>,
    cmd_tx: tokio::sync::mpsc::Sender<ClientMessage>,
    cancel_handle: CancelHandle,
    /// Unique client identifier (#352).
    client_id: u64,
    /// For in-reader handling of `tool_result` — see handle_client.
    client_tool_registry: super::uds_ext_protocol::ClientToolRegistry,
    /// RAII guard — decrements `live_clients` and sends `Disconnected` on drop.
    _guard: ClientGuard,
}

async fn handle_client(args: ClientHandlerArgs) {
    let ClientHandlerArgs {
        stream,
        mut broadcast_rx,
        mut targeted_rx,
        cmd_tx,
        cancel_handle,
        client_id,
        client_tool_registry,
        _guard,
    } = args;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let (reader, mut writer) = tokio::io::split(stream);
    let mut lines = BufReader::new(reader).lines();

    // Writer task: multiplex shared broadcast events AND per-client
    // targeted events (currently just `execute_tool` from forwarder
    // tasks) onto the client's socket. Targeted events never fan out
    // to other clients.
    let writer_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                b = broadcast_rx.recv() => match b {
                    Ok(line) => {
                        if writer.write_all(line.as_bytes()).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("client lagged by {n} events");
                        let msg = format!(
                            "{{\"type\":\"error\",\"message\":\"dropped {} events — use get_messages to re-sync\"}}\n",
                            n
                        );
                        if writer.write_all(msg.as_bytes()).await.is_err() {
                            break;
                        }
                    }
                },
                t = targeted_rx.recv() => match t {
                    Some(line) if writer.write_all(line.as_bytes()).await.is_err() => {
                        break;
                    }
                    Some(_) => {}
                    None => {
                        // Sender side closed (client registry
                        // entry dropped). Fall through — broadcast
                        // may still be delivering.
                    }
                },
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
        // Intercept `tool_result` inline — the dispatch loop is
        // single-threaded and is blocked waiting for the agent's
        // in-flight prompt (which in turn is waiting for *this* very
        // tool_result). Routing through cmd_tx would deadlock. We
        // resolve the pending oneshot directly against the shared
        // tool registry; the agent's UdsTool::execute wakes and the
        // turn resumes.
        if let Some(parsed) = try_intercept_tool_result(&line) {
            super::uds_ext_protocol::handle_tool_result(super::uds_ext_protocol::ToolResultArgs {
                client_id,
                tool_call_id: &parsed.tool_call_id,
                content: &parsed.content,
                is_error: parsed.is_error,
                registry: &client_tool_registry,
            });
            continue;
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
#[path = "uds_multi_prompt.rs"]
mod uds_multi_prompt;
use uds_multi_prompt::try_intercept_tool_result;
pub(crate) use uds_multi_prompt::{PromptArgsBroadcast, run_agent_prompt_broadcast};
#[cfg(test)]
#[path = "uds_multi_interception_tests.rs"]
mod interception_tests;
