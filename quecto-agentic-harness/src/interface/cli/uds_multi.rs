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
    DispatchCtx, LineResult, MAX_FRAME_PAYLOAD_BYTES, dispatch_command,
    emit_event_to_broadcast_or_writer, inject_system_prompt, is_cancel_command, parse_line,
    remove_injected_system_prompt,
};
use super::uds_cancel::{CancelHandle, CancelSlot, fire_cancel};
pub(super) use super::uds_multi_accept::{AcceptLoopArgs, spawn_accept_loop};
use super::uds_session::AgentSession;
pub(crate) use super::uds_snapshots::{ConversationSnapshot, StateSnapshot};
#[cfg(test)]
pub(crate) use super::uds_snapshots::{
    build_get_messages_line, build_get_state_line, build_get_subagents_line,
};
use super::uds_snapshots::{
    refresh_conversation_snapshot, refresh_session_stats_snapshot, refresh_state_snapshot,
    refresh_tool_catalogue_snapshot,
};

// ─── Constants ────────────────────────────────────────────────────────────────

/// Maximum number of concurrent client connections.
pub(super) const MAX_CLIENTS: u32 = 64;

/// Broadcast channel capacity for UDS event delivery.
/// Shared between the early-creation path (workflow) and the default path.
pub(super) const BROADCAST_CHANNEL_CAPACITY: usize = 256;

/// Atomic counter for assigning unique client IDs (#352).
pub(super) static NEXT_CLIENT_ID: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

/// Shared "agent is mid-turn" flag (#828). Set by the dispatch loop for the
/// duration of `agent.process()` (via [`BusyGuard`]), read by the accept loop.
/// The connect-time conversation snapshot is pushed to a newly-connected client
/// ONLY when this is `true` — i.e. the agent is busy and cannot answer a
/// `get_messages` promptly via the (blocked) single dispatch loop. When idle the
/// dispatch loop answers `get_messages` itself in FIFO order, so no unsolicited
/// bytes are written and clients that don't ask see no protocol change.
pub(crate) type BusyFlag = std::sync::Arc<std::sync::atomic::AtomicBool>;

/// RAII guard: marks the agent busy for the duration of a turn and clears the
/// flag on drop (normal completion, early return, or panic) (#828).
pub(crate) struct BusyGuard(BusyFlag);

impl BusyGuard {
    pub(crate) fn new(flag: &BusyFlag) -> Self {
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
        Self(flag.clone())
    }
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

// ─── Types ────────────────────────────────────────────────────────────────────

pub(super) struct MultiClientArgs<'a> {
    pub agent: AgentLoopImpl,
    pub base_dir: &'a std::path::Path,
    pub workspace: &'a std::path::Path,
    pub messages: Vec<Message>,
    pub model: String,
    pub session_key: String,
    pub ephemeral: bool,
    pub system_prompt: String,
    /// Shared tool catalogue snapshot for get_tool_catalogue.
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
    pub container_registry:
        Option<crate::infrastructure::tools::container_registry::ContainerRegistry>,
    /// Shared workflow state for auto-nudge injection (#562).
    pub workflow_state: Option<crate::interface::shared::WorkflowStateHandle>,
    /// Workflow config (auto_continue, completion_nudge flags).
    pub workflow_config: Option<crate::domain::workflow::WorkflowConfig>,
    /// Pre-created broadcast channel for workflow event emission (#598).
    pub broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    pub provider_reload: Option<&'a mut super::provider_reload::ProviderReload>,
    pub provider_reload_inputs: Option<&'a super::provider_reload::ProviderReloadInputs>,
    pub last_persisted_message_index: usize,
}

/// A command line from a client.
pub(super) struct ClientCommand {
    pub(super) line: String,
    /// Unique client identifier for per-client tool routing (#352).
    pub(super) client_id: u64,
}

/// Sentinel: a client disconnected.
pub(super) struct ClientDisconnected {
    /// Which client disconnected (#352).
    pub(super) client_id: u64,
}

/// Messages from client reader tasks to the dispatch loop.
pub(super) enum ClientMessage {
    Command(ClientCommand),
    Disconnected(ClientDisconnected),
}

/// RAII guard that decrements `live_clients` on drop (normal exit or panic).
pub(super) struct ClientGuard {
    pub(super) live_clients: std::sync::Arc<std::sync::atomic::AtomicU32>,
    pub(super) cmd_tx: tokio::sync::mpsc::Sender<ClientMessage>,
    /// Unique client identifier for per-client tool tracking (#352).
    pub(super) client_id: u64,
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
    let container_registry = args.container_registry;
    let wf_state = args.workflow_state;
    let wf_config = args.workflow_config;
    let pre_broadcast_tx = args.broadcast_tx;
    let provider_reload = args.provider_reload;
    let provider_reload_inputs = args.provider_reload_inputs;
    let last_persisted_message_index = args.last_persisted_message_index;
    let MultiClientArgs {
        mut agent,
        base_dir,
        workspace,
        mut messages,
        model,
        mut session_key,
        ephemeral,
        system_prompt,
        ..
    } = args;

    inject_system_prompt(&mut messages, &system_prompt);

    // Shared pre-turn conversation snapshot (#828): initialized with the starting
    // messages (same shape as a normal get_messages response, i.e. including the
    // injected system prompt), refreshed by the dispatch loop at each turn
    // boundary, and read by the accept loop to serve newly-connected clients
    // immediately — even while the dispatch loop is busy mid-turn.
    let mut initial_conversation_snapshot =
        super::uds_snapshots::ConversationSnapshotData::from_messages(messages.clone());
    initial_conversation_snapshot
        .set_spill_store(agent.spill_store().cloned(), session_key.clone());
    let conversation_snapshot: ConversationSnapshot =
        std::sync::Arc::new(tokio::sync::RwLock::new(initial_conversation_snapshot));

    let mut agent_session = AgentSession::new(model, session_key.clone());
    let initial_state = agent_session.state_snapshot(
        messages.len(),
        None,
        agent.max_context_tokens(),
        agent.effort().map(|l| l.as_str().to_string()),
    );
    let state_snapshot: StateSnapshot =
        std::sync::Arc::new(tokio::sync::RwLock::new(initial_state));
    let execution_state: super::uds_execution_state::ExecutionStateHandle =
        std::sync::Arc::new(std::sync::Mutex::new(Default::default()));
    let session_stats_snapshot = std::sync::Arc::new(tokio::sync::RwLock::new(
        super::uds_session::compute_session_stats_with_usage(
            &session_key,
            &messages,
            agent_session.usage_snapshot(),
            agent_session.context_tokens(),
            agent.max_context_tokens(),
        ),
    ));
    let tool_catalogue_snapshot = std::sync::Arc::new(tokio::sync::RwLock::new(
        agent
            .tool_catalogue_entries()
            .into_iter()
            .map(|entry| serde_json::to_value(entry).unwrap_or_default())
            .collect(),
    ));

    // Shared mid-turn flag (#828): gates the connect-time snapshot push so it
    // only fires while the agent is busy (see `BusyFlag`).
    let busy: BusyFlag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    // Use the pre-created broadcast channel when available (workflow emitter
    // is already wired to it), otherwise create a fresh one (#598).
    let broadcast_tx = pre_broadcast_tx
        .unwrap_or_else(|| tokio::sync::broadcast::channel::<String>(BROADCAST_CHANNEL_CAPACITY).0);
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel::<ClientMessage>(256);
    let cancel_handle: CancelHandle = std::sync::Arc::new(std::sync::Mutex::new(CancelSlot::Idle));
    let turn_control: super::uds_cancel::TurnControlHandle = std::sync::Arc::default();
    let live_clients = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));

    let client_tool_registry = super::uds_ext_protocol::new_client_tool_registry();

    let accept_task = spawn_accept_loop(AcceptLoopArgs {
        listener,
        broadcast_tx: broadcast_tx.clone(),
        cmd_tx: cmd_tx.clone(),
        cancel_handle: cancel_handle.clone(),
        turn_control: turn_control.clone(),
        live_clients: live_clients.clone(),
        client_tool_registry: client_tool_registry.clone(),
        conversation_snapshot: conversation_snapshot.clone(),
        state_snapshot: state_snapshot.clone(),
        execution_state: execution_state.clone(),
        session_stats_snapshot: session_stats_snapshot.clone(),
        tool_catalogue_snapshot: tool_catalogue_snapshot.clone(),
        busy: busy.clone(),
        subagent_registry: subagent_registry.clone(),
        workflow_state: wf_state.clone(),
        workspace_path: workspace.to_path_buf(),
    });

    // Drop our clone so cmd_rx closes when all client senders (accept loop)
    // are gone.  The accept loop's clone keeps the channel open while
    // it runs — the `!persist` guard in `run_dispatch_loop` controls shutdown.
    drop(cmd_tx);

    let mut ctx = DispatchCtx {
        // Multi-client replies stream via broadcast; each client's writer
        // task re-frames per its own negotiated connection mode (#1059).
        wire_mode: super::uds_wire::ConnectionWireMode::legacy(),
        base_dir,
        agent: &mut agent,
        messages: &mut messages,
        conversation_snapshot: conversation_snapshot.clone(),
        state_snapshot: state_snapshot.clone(),
        execution_state: execution_state.clone(),
        session_stats_snapshot: session_stats_snapshot.clone(),
        tool_catalogue_snapshot: tool_catalogue_snapshot.clone(),
        busy: busy.clone(),
        session: &mut agent_session,
        stdout: None,
        session_key: &mut session_key,
        session_store,
        ephemeral,
        system_prompt: &system_prompt,
        cancel_handle,
        turn_control,
        broadcast_tx: Some(broadcast_tx),
        _ext_registry: ext_registry,
        client_tool_registry: client_tool_registry.clone(),
        current_client_id: 0,
        subagent_registry,
        container_registry,
        notification_rx,
        workflow_state: wf_state.clone(),
        workflow_config: wf_config,
        provider_reload,
        provider_reload_inputs,
        last_persisted_message_index,
        durable_prefix_dirty: false,
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
                let (dedupe_ref, _) = notif.await_dedupe_key();
                tracing::info!(%agent_id, sequence, "recording subagent completion note");
                // Auto-await dedupe: if a manual `await` already reported this
                // terminal completion (flag set on the registry entry), CONSUME
                // the flag and SKIP both the passive note enqueue and the
                // SubagentNotification emit — the parent already has the result.
                // The SubagentStateChanged panel update below still fires. This is
                // race-free because the dispatch loop is single-threaded: the
                // await tool call set the flag before this queued notification is
                // processed.
                let suppress =
                    crate::infrastructure::tools::subagent_registry::consume_await_dedupe(
                        &ctx.subagent_registry,
                        &dedupe_ref,
                    );
                let mut should_deliver = false;
                if !suppress {
                    // Auto-await (#816): enqueue the one-line note for delivery at
                    // the parent's NEXT idle boundary.
                    // `enqueue_subagent_notification` records the dedupe sequence
                    // internally and returns whether this completion is new — so we
                    // don't also call `record_subagent_notification` (that would
                    // double-dedupe).
                    let outcome = ctx.session.enqueue_subagent_notification(
                        agent_id.clone(),
                        sequence,
                        notif.to_message(),
                        notif.is_completion(),
                    );
                    // #1082 review round 2: only a retained note is announced;
                    // Duplicate means already delivered, Dropped means both
                    // buffers are full — the sequence stays retryable and the
                    // monitor-side retention re-delivers critical alerts.
                    if outcome.is_retained() {
                        should_deliver = true;
                        let ev = AgentEvent::SubagentNotification {
                            agent_id: agent_id.clone(),
                            sequence,
                            message: notif.to_message(),
                        };
                        emit_event_to_broadcast_or_writer(ctx, &ev).await;
                    }
                }
                // Broadcast state_changed event to all UDS clients (#524).
                let list = super::protocol::build_subagent_info_list(&ctx.subagent_registry);
                let ev = AgentEvent::SubagentStateChanged { subagents: list };
                emit_event_to_broadcast_or_writer(ctx, &ev).await;
                // Auto-await (#816): this branch runs only while the parent is IDLE
                // (mid-turn completions are buffered and drained after that turn).
                // Deliver the just-enqueued note NOW so the parent processes it and
                // can CONTINUE its task — e.g. score a poem the child just wrote —
                // instead of stalling until the next user message. The TUI defers
                // DISPLAY of the note until the parent is idle, so acting here never
                // splits an in-flight response.
                if should_deliver {
                    super::uds::drain_pending_and_nudge(ctx).await;
                }
            }
        }
        // Turn boundary: refresh the shared snapshot so a newly-connected client
        // is served the up-to-date prior conversation on connect (#828).
        refresh_conversation_snapshot(ctx).await;
        refresh_state_snapshot(ctx).await;
        refresh_session_stats_snapshot(ctx).await;
        refresh_tool_catalogue_snapshot(ctx).await;
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
                LineResult::ParseError(e) if e.is_empty() => {}
                LineResult::ParseError(e) => {
                    // #994 criterion 2: preserve the detailed serde parse-error
                    // text, consistent with the single-client loop
                    // (`uds::run_command_loop`), rather than substituting a
                    // generic placeholder string.
                    let ev = AgentEvent::err(None, "parse_error", e);
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
    let before: Vec<serde_json::Value> = ctx
        .agent
        .tool_catalogue_entries()
        .into_iter()
        .map(|entry| serde_json::to_value(entry).unwrap_or_default())
        .collect();
    let removed =
        super::uds_ext_protocol::handle_client_disconnect(client_id, &ctx.client_tool_registry);
    if !removed.is_empty() {
        ctx.agent.unregister_uds_tools_for_client(client_id);
        let after: Vec<serde_json::Value> = ctx
            .agent
            .tool_catalogue_entries()
            .into_iter()
            .map(|entry| serde_json::to_value(entry).unwrap_or_default())
            .collect();
        {
            let mut snapshot = ctx.tool_catalogue_snapshot.write().await;
            *snapshot = after.clone();
        }
        let ev = AgentEvent::ToolCatalogueChanged {
            changed_tools: removed,
            before,
            after,
            reason: "client_disconnect".to_string(),
        };
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
    }
}

// ─── Per-client handler ───────────────────────────────────────────────────────

/// Arguments for [`handle_client`].
pub(super) struct ClientHandlerArgs {
    pub(super) stream: tokio::net::UnixStream,
    pub(super) broadcast_rx: tokio::sync::broadcast::Receiver<String>,
    /// Per-client targeted event stream — receives events addressed
    /// to this client only (currently just `execute_tool` from
    /// forwarder tasks). Writer_task selects over this AND
    /// broadcast_rx so targeted events aren't visible to other
    /// clients.
    pub(super) targeted_rx: tokio::sync::mpsc::Receiver<String>,
    pub(super) cmd_tx: tokio::sync::mpsc::Sender<ClientMessage>,
    pub(super) cancel_handle: CancelHandle,
    /// Shared abort/steer control flags (#895/#896).
    pub(super) turn_control: super::uds_cancel::TurnControlHandle,
    /// Unique client identifier (#352).
    pub(super) client_id: u64,
    /// For in-reader handling of `tool_result` — see handle_client.
    pub(super) client_tool_registry: super::uds_ext_protocol::ClientToolRegistry,
    /// Live conversation ledger. Updated as messages are appended during a turn,
    /// allowing read-only get_message lookups to bypass the blocked dispatcher.
    pub(super) conversation_snapshot: ConversationSnapshot,
    /// Sub-agent registry, read mid-turn to serve `get_subagents` and forward
    /// child-targeted `sync` off the blocked dispatcher (spike).
    pub(super) subagent_registry:
        Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
    /// RAII guard — decrements `live_clients` and sends `Disconnected` on drop.
    pub(super) _guard: ClientGuard,
}

pub(super) async fn handle_client(args: ClientHandlerArgs) {
    let ClientHandlerArgs {
        stream,
        mut broadcast_rx,
        mut targeted_rx,
        cmd_tx,
        cancel_handle,
        turn_control,
        client_id,
        client_tool_registry,
        conversation_snapshot,
        subagent_registry,
        _guard,
    } = args;
    use tokio::io::BufReader;

    let (reader, mut writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(reader);

    // Negotiated per-connection framing (#1059): the reader loop below
    // records the client's detected framing; the writer task replies in it.
    // Until the client has spoken, writes fall back to legacy NDJSON — safe
    // because framed clients sniff each incoming message during the
    // deprecation window (see `uds_wire` module docs).
    let wire_mode = super::uds_wire::ConnectionWireMode::default();
    let wire_mode_for_writer = wire_mode.clone();

    // Writer task: multiplex shared broadcast events AND per-client
    // targeted events (currently just `execute_tool` from forwarder
    // tasks) onto the client's socket. Targeted events never fan out
    // to other clients.
    let writer_task = tokio::spawn(async move {
        let mode = wire_mode_for_writer;
        loop {
            tokio::select! {
                b = broadcast_rx.recv() => match b {
                    Ok(line) => {
                        if super::uds_wire::write_event_line(&mut writer, &line, &mode).await.is_err() {
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
                        if super::uds_wire::write_event_line(&mut writer, &msg, &mode).await.is_err() {
                            break;
                        }
                    }
                },
                t = targeted_rx.recv() => match t {
                    Some(line) if super::uds_wire::write_event_line(&mut writer, &line, &mode).await.is_err() => {
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
    // Each message is sniffed as a length-prefixed frame or a legacy NDJSON
    // line (#1059, deprecation window). Over-cap messages are rejected while
    // reading (bounded memory, #1003) with the connection kept usable; a peer
    // speaking neither framing is an explicit version mismatch and the
    // connection closes — never a silent misparse or a hang.
    loop {
        let incoming = match quecto_line_io::read_frame_or_legacy_line(
            &mut reader,
            MAX_FRAME_PAYLOAD_BYTES,
        )
        .await
        {
            Ok(Some(incoming)) => incoming,
            Ok(None) => break,
            Err(e @ quecto_line_io::FrameError::Oversized { .. }) => {
                tracing::warn!(client_id, "dropping over-cap message from client: {e}");
                continue;
            }
            Err(e @ quecto_line_io::FrameError::VersionMismatch { .. }) => {
                tracing::warn!(client_id, "closing client connection: {e}");
                break;
            }
            Err(e) => {
                tracing::warn!(client_id, error = %e, "client reader loop exiting on I/O error");
                break;
            }
        };
        let (mode, bytes) = match incoming {
            quecto_line_io::Incoming::Frame(b) => (quecto_line_io::WireMode::Framed, b),
            quecto_line_io::Incoming::LegacyLine(b) => (quecto_line_io::WireMode::LegacyLine, b),
        };
        wire_mode.record(mode);
        // Reuse the payload `Vec`'s allocation on the common valid-UTF-8 path;
        // only pay a copy for the lossy fallback on malformed input.
        let line = String::from_utf8(bytes)
            .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
        let trimmed = line.trim();

        if is_cancel_command(trimmed) {
            // Record operator intent BEFORE firing the cancel so the post-cancel
            // idle drain cannot observe the cancel and run a nudge before the
            // abort/steer flag lands (#895/#896).
            if super::uds::is_abort_command(trimmed) {
                turn_control.mark_abort();
            } else if super::uds::is_steer_command(trimmed) {
                turn_control.mark_steer();
            }
            fire_cancel(&cancel_handle);
        }
        if !super::uds_reader_dispatch::dispatch(super::uds_reader_dispatch::ReaderDispatchCtx {
            line,
            snapshot: &conversation_snapshot,
            registry: &client_tool_registry,
            subagent_registry: &subagent_registry,
            client_id,
            cmd_tx: &cmd_tx,
        })
        .await
        {
            break;
        }
    }

    writer_task.abort();
    // `_guard` is dropped here (or on panic), which decrements live_clients
    // and sends the Disconnected sentinel.
}

// ─── Broadcast prompt execution ───────────────────────────────────────────────

// Re-exported for the auto-await dedupe unit tests (uds_subagent_notify_tests).
#[cfg(test)]
pub(in crate::interface::cli) use super::uds_cancel::forward_notification_broadcast;
#[cfg(test)]
#[path = "uds_multi_accept_loop_tests.rs"]
mod accept_loop_tests;
#[cfg(test)]
#[path = "uds_multi_cov2_tests.rs"]
mod cov2_tests;
#[cfg(test)]
#[path = "uds_multi_cov_tests.rs"]
mod cov_tests;
#[cfg(test)]
#[path = "uds_multi_interception_tests.rs"]
mod interception_tests;
#[cfg(test)]
#[path = "uds_multi_926_wake_tests.rs"]
mod issue_926_wake_tests;
#[cfg(test)]
#[path = "uds_994_tests.rs"]
mod issue_994_tests;
#[cfg(test)]
#[path = "uds_paged_history_tests.rs"]
mod paged_history_tests;
#[cfg(test)]
#[path = "uds_snapshot_tests.rs"]
mod snapshot_tests;
