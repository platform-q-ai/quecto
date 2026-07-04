use super::protocol::{AgentCommand, AgentEvent, StreamingBehavior};
use super::uds_cancel::{
    CancelHandle, CancelSlot, PromptArgs, PromptMessageArgs, PromptOutcome, TurnControl,
    TurnControlHandle, arm_cancel, disarm_cancel, run_agent_message, run_agent_prompt,
};
use super::uds_multi::{MultiClientArgs, PromptArgsBroadcast, run_agent_prompt_broadcast};
use super::uds_query::query_response_data;
use super::uds_session::{AgentSession, clear_conversation, rewind_to_message_index};
#[cfg(test)]
use super::uds_session::{
    compute_session_stats, compute_session_stats_with_usage, messages_tail_json,
};
use super::uds_workflow_nudge::{workflow_nudge_message, workflow_progress_fingerprint};
use crate::application::agent_loop::AgentLoopImpl;
use crate::domain::message::{Message, Role};
use crate::domain::session::{Session, SessionStore};
use crate::infrastructure::model_registry::ModelRegistry;
use crate::infrastructure::persistence::session_store::FileSessionStore;

type ExtRegistry = std::sync::Arc<
    std::sync::Mutex<crate::infrastructure::extensions::registry::ExtensionRegistry>,
>;

pub struct UdsLoopArgs<'a> {
    pub agent: AgentLoopImpl,
    pub base_dir: &'a std::path::Path,
    pub session_key: String,
    pub model: String,
    pub ephemeral: bool,
    pub system_prompt: String,
    pub socket_path: std::path::PathBuf,
    /// `None` = multi-client mode. `Some` = single-client mode (tests).
    pub socket_override: Option<std::os::unix::net::UnixStream>,
    pub session_store_override: Option<Box<dyn SessionStore + 'static>>,
    pub ext_registry: Option<ExtRegistry>,
    pub persist: bool,
    pub notification_rx: Option<crate::infrastructure::tools::subagent_registry::NotificationRx>,
    pub subagent_registry:
        Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
    pub workflow_state: Option<crate::interface::shared::WorkflowStateHandle>, // #562
    pub workflow_config: Option<crate::domain::workflow::WorkflowConfig>,      // #562
    /// Pre-created broadcast channel for workflow event emission (#598).
    pub broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    pub provider_reload: Option<&'a mut super::provider_reload::ProviderReload>,
    pub provider_reload_inputs: Option<&'a super::provider_reload::ProviderReloadInputs>,
}
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
pub(crate) use super::uds_socket::reap_stale_sockets;
use super::uds_socket::{SocketGuard, bind_secure_socket};

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
        persist,
        notification_rx,
        subagent_registry,
        workflow_state,
        workflow_config,
        broadcast_tx,
        provider_reload,
        provider_reload_inputs,
    } = args;
    let file_store;
    let session_store: &dyn SessionStore = match session_store_override {
        Some(ref s) => s.as_ref(),
        None => {
            file_store = FileSessionStore::new(base_dir);
            &file_store
        }
    };
    let loaded_session = match load_session(session_store, &session_key, ephemeral).await {
        Ok(m) => m,
        Err(err) => {
            eprintln!("failed to load session: {err}");
            return 1;
        }
    };
    let loaded_message_count = loaded_session.messages.len();
    let messages = loaded_session.messages;
    if let (Some(ws), Some(persisted)) = (&workflow_state, loaded_session.workflow_run) {
        if let Ok(mut engine) = ws.lock() {
            engine.restore_run(persisted);
        }
    }

    if let Some(std_stream) = socket_override {
        // Single-client path: backward-compatible with existing tests.
        single_client_loop(
            SingleClientArgs {
                agent,
                base_dir,
                messages,
                model,
                session_key,
                ephemeral,
                system_prompt,
                ext_registry,
                workflow_state,
                provider_reload,
                provider_reload_inputs,
                last_persisted_message_index: loaded_message_count,
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
                base_dir,
                messages,
                model,
                session_key,
                ephemeral,
                system_prompt,
                ext_registry,
                persist,
                notification_rx,
                subagent_registry,
                workflow_state,
                workflow_config,
                broadcast_tx,
                provider_reload,
                provider_reload_inputs,
                last_persisted_message_index: loaded_message_count,
            },
            listener,
            session_store,
        )
        .await
    }
}

struct SingleClientArgs<'a> {
    agent: AgentLoopImpl,
    base_dir: &'a std::path::Path,
    messages: Vec<Message>,
    model: String,
    session_key: String,
    ephemeral: bool,
    system_prompt: String,
    ext_registry: Option<ExtRegistry>,
    workflow_state: Option<crate::interface::shared::WorkflowStateHandle>,
    provider_reload: Option<&'a mut super::provider_reload::ProviderReload>,
    provider_reload_inputs: Option<&'a super::provider_reload::ProviderReloadInputs>,
    last_persisted_message_index: usize,
}

async fn single_client_loop(
    args: SingleClientArgs<'_>,
    std_stream: std::os::unix::net::UnixStream,
    session_store: &dyn SessionStore,
) -> i32 {
    let SingleClientArgs {
        agent,
        base_dir,
        mut messages,
        model,
        mut session_key,
        ephemeral,
        system_prompt,
        ext_registry,
        workflow_state,
        provider_reload,
        provider_reload_inputs,
        last_persisted_message_index,
    } = args;
    std_stream
        .set_nonblocking(true)
        .expect("set_nonblocking failed for test socket");
    let tokio_stream = tokio::net::UnixStream::from_std(std_stream).expect("std→tokio UnixStream");
    let (r, w) = tokio::io::split(tokio_stream);
    let reader: Box<dyn tokio::io::AsyncRead + Send + Unpin> = Box::new(r);
    let mut writer: Box<dyn tokio::io::AsyncWrite + Send + Unpin> = Box::new(w);

    inject_system_prompt(&mut messages, &system_prompt);

    let mut agent_session = AgentSession::new(model, session_key.clone());
    let max_context_tokens = agent.max_context_tokens();
    let initial_stats = super::uds_session::compute_session_stats(&session_key, &messages);

    run_command_loop(
        reader,
        &mut DispatchCtx {
            base_dir,
            agent: &mut { agent },
            messages: &mut messages,
            conversation_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
            state_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(
                agent_session.state_snapshot(0, None, max_context_tokens),
            )),
            session_stats_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(initial_stats)),
            extension_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
            busy: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            session: &mut agent_session,
            stdout: &mut *writer,
            session_key: &mut session_key,
            session_store,
            ephemeral,
            system_prompt: &system_prompt,
            cancel_handle: std::sync::Arc::new(std::sync::Mutex::new(CancelSlot::Idle)),
            turn_control: std::sync::Arc::<TurnControl>::default(),
            broadcast_tx: None,
            ext_registry,
            client_tool_registry: super::uds_ext_protocol::new_client_tool_registry(),
            current_client_id: 0,
            subagent_registry: None,
            notification_rx: None,
            workflow_state: workflow_state.clone(),
            workflow_config: None,
            provider_reload,
            provider_reload_inputs,
            last_persisted_message_index,
        },
    )
    .await;

    if !ephemeral && !session_key.is_empty() {
        remove_injected_system_prompt(&mut messages, &system_prompt);
        let session = Session {
            key: session_key,
            messages: std::mem::take(&mut messages),
            workflow_run: workflow_state
                .as_ref()
                .and_then(|ws| ws.lock().ok().and_then(|engine| engine.persisted_run())),
        };
        let _ = session_store.save(&session).await;
    }
    0
}

pub(crate) fn inject_system_prompt(messages: &mut Vec<Message>, prompt: &str) {
    if prompt.is_empty() {
        return;
    }
    let has_real_system = messages
        .first()
        .is_some_and(|m| m.role == Role::System && !m.is_manifest);
    if !has_real_system {
        messages.insert(0, Message::system(prompt.to_string()));
    }
}

pub(crate) fn remove_injected_system_prompt(messages: &mut Vec<Message>, prompt: &str) {
    if prompt.is_empty() {
        return;
    }
    let is_injected_prompt = messages.first().is_some_and(|m| {
        m.role == Role::System
            && !m.is_manifest
            && (m.content == prompt || m.content.starts_with(prompt))
    });
    if is_injected_prompt {
        messages.remove(0);
    }
}

pub(super) const MAX_LINE_BYTES: usize = 1024 * 1024;

pub(super) fn is_cancel_command(trimmed: &str) -> bool {
    is_abort_command(trimmed) || is_steer_command(trimmed)
}

/// Reader-side classification so the eager cancel and the abort/steer control
/// flag are set together, before the command is dispatched (#895/#896).
pub(super) fn is_abort_command(trimmed: &str) -> bool {
    trimmed.contains("\"type\":\"abort\"")
}

pub(super) fn is_steer_command(trimmed: &str) -> bool {
    trimmed.contains("\"type\":\"steer\"")
}

pub(super) enum LineResult {
    Command(AgentCommand),
    ParseError(String),
    LineTooLong,
}

pub(super) fn parse_line(line: &str) -> LineResult {
    if line.len() > MAX_LINE_BYTES {
        return LineResult::LineTooLong;
    }
    let line = line.trim();
    if line.is_empty() {
        return LineResult::ParseError(String::new());
    }
    match super::protocol::parse_command_line(line) {
        Ok(c) => LineResult::Command(c),
        Err(e) => LineResult::ParseError(e),
    }
}

async fn run_command_loop(
    reader: Box<dyn tokio::io::AsyncRead + Send + Unpin>,
    ctx: &mut DispatchCtx<'_>,
) {
    use super::uds_reader::{RawLine, spawn_reader_task};

    let cancel_for_reader = std::sync::Arc::clone(&ctx.cancel_handle);
    let control_for_reader = std::sync::Arc::clone(&ctx.turn_control);

    let (reader_task, mut rx) = spawn_reader_task(reader, cancel_for_reader, control_for_reader);

    loop {
        let raw = match rx.recv().await {
            Some(Some(RawLine::Line(l))) => l,
            Some(Some(RawLine::TooLong)) => {
                let ev = AgentEvent::err(None, "parse_error", "line exceeds 1 MiB limit");
                emit_event_to_broadcast_or_writer(ctx, &ev).await;
                continue;
            }
            _ => break,
        };

        match parse_line(&raw) {
            LineResult::LineTooLong => {
                let ev = AgentEvent::err(None, "parse_error", "line exceeds 1 MiB limit");
                emit_event_to_broadcast_or_writer(ctx, &ev).await;
            }
            LineResult::ParseError(e) if e.is_empty() => {}
            LineResult::ParseError(e) => {
                let ev = AgentEvent::err(None, "parse_error", e);
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

mod uds_session_load;
use uds_session_load::load_session;

pub(crate) struct DispatchCtx<'a> {
    pub base_dir: &'a std::path::Path,
    pub agent: &'a mut AgentLoopImpl,
    pub messages: &'a mut Vec<Message>,
    pub conversation_snapshot: super::uds_multi::ConversationSnapshot, // #828
    pub state_snapshot: super::uds_multi::StateSnapshot,               // #837
    pub session_stats_snapshot: super::uds_snapshots::SessionStatsSnapshot, // #880
    pub extension_snapshot: super::uds_extensions::ExtensionSnapshot,  // #880
    pub busy: super::uds_multi::BusyFlag,                              // #828
    pub session: &'a mut AgentSession,
    pub stdout: &'a mut (dyn tokio::io::AsyncWrite + Send + Unpin),
    pub session_key: &'a mut String,
    pub session_store: &'a dyn SessionStore,
    pub ephemeral: bool,
    pub system_prompt: &'a str,
    pub cancel_handle: CancelHandle,
    /// Cross-task control flags for abort/steer vs workflow auto-continue
    /// (#895/#896). Set by the reader task, read at the idle boundary.
    pub turn_control: TurnControlHandle,
    pub broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    pub ext_registry: Option<ExtRegistry>,
    pub client_tool_registry: super::uds_ext_protocol::ClientToolRegistry,
    pub current_client_id: u64,
    pub subagent_registry:
        Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
    pub notification_rx: Option<crate::infrastructure::tools::subagent_registry::NotificationRx>,
    pub workflow_state: Option<crate::interface::shared::WorkflowStateHandle>, // #562
    pub workflow_config: Option<crate::domain::workflow::WorkflowConfig>,      // #562
    pub provider_reload: Option<&'a mut super::provider_reload::ProviderReload>,
    pub provider_reload_inputs: Option<&'a super::provider_reload::ProviderReloadInputs>,
    pub last_persisted_message_index: usize,
}

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
    super::uds_reload::poll_provider_reload_for_ctx(ctx).await;
    let resolved_model = match resolve_set_model_target(args.model, args.provider, args.model_id) {
        Ok(m) => m,
        Err(msg) => {
            let ev = AgentEvent::err(args.id.as_deref(), &args.type_name, msg);
            emit_event_to_broadcast_or_writer(ctx, &ev).await;
            return false;
        }
    };
    // #935: re-derive the per-model cap so a model switch re-clamps subsequent
    // turns; set_model takes both atomically so model and cap cannot diverge.
    let cap = ModelRegistry::model_cap_from_base_dir(ctx.base_dir, &resolved_model);
    ctx.agent.set_model(resolved_model.clone(), cap);
    ctx.session.set_model(resolved_model);
    tracing::debug!(new_model = %ctx.session.model(), "UDS: model switched");
    let ev = AgentEvent::ok(args.id.as_deref(), &args.type_name, None);
    emit_event_to_broadcast_or_writer(ctx, &ev).await;
    false
}

fn session_summary_to_json(summary: &crate::domain::session::SessionSummary) -> serde_json::Value {
    serde_json::json!({
        "key": summary.key,
        "title": display_title(&summary.title),
        "messageCount": summary.message_count,
        "updatedUnixSecs": summary.updated_unix_secs,
        "updatedAt": summary.updated_unix_secs,
    })
}

/// Apply display policy to a raw title: blank → "(untitled)", else truncate to 50 chars.
fn display_title(raw: &str) -> String {
    const MAX_CHARS: usize = 50;
    if raw.is_empty() {
        return "(untitled)".to_string();
    }
    if raw.chars().count() <= MAX_CHARS {
        return raw.to_string();
    }
    let mut out: String = raw.chars().take(MAX_CHARS).collect();
    out.push('…');
    out
}

/// Returns `Some(bool)` if handled, `None` to fall through to the main match.
async fn dispatch_fieldless_command(cmd: &AgentCommand, ctx: &mut DispatchCtx<'_>) -> Option<bool> {
    let id = cmd.id();
    let tn = cmd.type_name();
    if matches!(cmd, AgentCommand::ListSessions { .. }) {
        let event = match ctx
            .session_store
            .list(Some(crate::domain::session::USER_CHAT_PREFIX))
            .await
        {
            Ok(sessions) => AgentEvent::ok(
                id,
                tn,
                Some(serde_json::json!({
                    "sessions": sessions
                        .iter()
                        .map(session_summary_to_json)
                        .collect::<Vec<_>>()
                })),
            ),
            Err(err) => AgentEvent::err(id, tn, err.to_string()),
        };
        emit_event_to_broadcast_or_writer(ctx, &event).await;
        return Some(false);
    }
    if let Some(data) = query_response_data(cmd, ctx) {
        let ev = AgentEvent::ok(id, tn, Some(data));
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
        return Some(false);
    }
    if matches!(cmd, AgentCommand::ClearHistory { .. }) {
        return Some(handle_clear_history(ctx, id, tn).await);
    }
    None
}

#[path = "uds_dispatch.rs"]
mod uds_dispatch;
pub(crate) use uds_dispatch::dispatch_command;
use uds_dispatch::handle_clear_history;

struct PromptCommand {
    id: Option<String>,
    type_name: String,
    message: String,
    streaming_behavior: Option<StreamingBehavior>,
}

async fn handle_prompt(ctx: &mut DispatchCtx<'_>, cmd: PromptCommand) -> bool {
    // A genuine `prompt` takes over from any stale steer gate. The reader marks
    // `steer_pending` from a loose `"type":"steer"` substring match, so a prompt
    // whose body merely quotes the protocol (or a line that fails to parse) could
    // otherwise leave the gate stuck `true`, permanently suppressing the
    // auto-continue nudge (#896 AC3). `handle_steer` is the only other clearer.
    ctx.turn_control.clear_steer();
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
            }
            None => {
                let msg = "agent is running; provide streamingBehavior";
                let ev = AgentEvent::err(id.as_deref(), &type_name, msg);
                emit_event_to_broadcast_or_writer(ctx, &ev).await;
            }
        }
        return false;
    }
    super::uds_reload::poll_provider_reload_for_ctx(ctx).await;
    let Some(cancel_rx) = arm_cancel(&ctx.cancel_handle) else {
        emit_pre_cancelled(ctx).await; // Stale abort (#483).
        drain_and_run_pending(ctx).await;
        return false;
    };
    let outcome = run_prompt_dispatch(ctx, message, cancel_rx).await;
    disarm_cancel(&ctx.cancel_handle);
    if matches!(outcome, PromptOutcome::Success) {
        let ev = AgentEvent::ok(id.as_deref(), &type_name, None);
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
    }
    drain_pending_and_nudge(ctx).await;
    // Persist after every turn so the conversation survives an ungraceful exit.
    if let Err(err) = uds_dispatch::persist_current_session(ctx).await {
        tracing::warn!("failed to persist session after turn: {err}");
    }
    false
}

/// Drain pending messages, then inject core workflow nudges while progress is advancing (#562).
pub(super) async fn drain_pending_and_nudge(ctx: &mut DispatchCtx<'_>) {
    // #895: abort = full stop. A pending abort (set by the reader before this
    // command's handler runs) suppresses workflow auto-continue and discards
    // queued work, so the bound workflow does NOT resume at this idle boundary —
    // it stays stopped until re-driven by a fresh prompt. Abort beats any steer.
    if ctx.turn_control.take_abort() {
        ctx.turn_control.clear_steer();
        ctx.session.drain_pending();
        return;
    }

    drain_and_run_pending(ctx).await;

    // Bounded so a misbehaving model that ignores the workflow isn't nudged forever.
    const MAX_WORKFLOW_NUDGES: usize = 128;
    for _ in 0..MAX_WORKFLOW_NUDGES {
        // #930: an abort that lands WHILE this auto-continue loop is mid-flight is
        // a full stop, exactly like one at the idle-drain entry above — discard
        // queued work and stop nudging. The entry guard only catches an abort that
        // arrived before the drain; without re-checking here, a workflow that was
        // already auto-continuing kept advancing past the abort (e.g. 5/17 → 9/17)
        // because the loop only broke on a pending steer or a no-progress turn.
        if ctx.turn_control.take_abort() {
            ctx.turn_control.clear_steer();
            ctx.session.drain_pending();
            return;
        }
        // #896: an explicit steer outranks the auto-continue nudge — yield so the
        // steered instruction is obeyed next instead of being overridden.
        if ctx.turn_control.is_steer_pending() {
            break;
        }
        let before = workflow_progress_fingerprint(ctx);
        let Some(message) = workflow_nudge_message(ctx) else {
            break;
        };
        ctx.session.enqueue_pending(message);
        drain_and_run_pending(ctx).await;
        let after = workflow_progress_fingerprint(ctx);
        if after == before {
            break;
        }
    }
}

async fn run_prompt_dispatch(
    ctx: &mut DispatchCtx<'_>,
    message: String,
    cancel_rx: tokio::sync::oneshot::Receiver<()>,
) -> PromptOutcome {
    let _busy = super::uds_multi::BusyGuard::new(&ctx.busy); // #828: gates connect-time snapshot
    if let Some(ref tx) = ctx.broadcast_tx {
        run_agent_prompt_broadcast(PromptArgsBroadcast {
            agent: ctx.agent,
            messages: ctx.messages,
            session: ctx.session,
            broadcast_tx: tx.clone(),
            message: Message::user(message),
            cancel_rx,
            notification_rx: &mut ctx.notification_rx,
            subagent_registry: &ctx.subagent_registry,
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
    }
}
async fn emit_pre_cancelled(ctx: &mut DispatchCtx<'_>) {
    emit_event_to_broadcast_or_writer(ctx, &AgentEvent::AgentStart).await;
    emit_event_to_broadcast_or_writer(ctx, &AgentEvent::AgentEnd { messages: vec![] }).await;
}
async fn drain_and_run_pending(ctx: &mut DispatchCtx<'_>) {
    let _busy = super::uds_multi::BusyGuard::new(&ctx.busy); // #828
    loop {
        // Coalesce a burst of buffered sub-agent completion notes into ONE
        // informational note at the idle flush (#894) — K separate notes would
        // send the parent into a catch-up loop. Per-agent dedup already happened
        // at enqueue; #816 deferral is preserved (this only runs at idle).
        let pending = super::uds_session::coalesce_pending(ctx.session.drain_pending());
        if pending.is_empty() {
            break;
        }
        for pending_msg in pending {
            let msg = pending_msg.into_message();
            let Some(rx) = arm_cancel(&ctx.cancel_handle) else {
                emit_pre_cancelled(ctx).await; // Stale abort (#483).
                continue; // Don't drop remaining messages — Fired consumed, next arm succeeds.
            };
            if let Some(ref tx) = ctx.broadcast_tx {
                let args = PromptArgsBroadcast {
                    agent: ctx.agent,
                    messages: ctx.messages,
                    session: ctx.session,
                    broadcast_tx: tx.clone(),
                    message: msg,
                    cancel_rx: rx,
                    notification_rx: &mut ctx.notification_rx,
                    subagent_registry: &ctx.subagent_registry,
                };
                run_agent_prompt_broadcast(args).await;
            } else {
                let args = PromptMessageArgs {
                    agent: ctx.agent,
                    messages: ctx.messages,
                    session: ctx.session,
                    stdout: ctx.stdout,
                    message: msg,
                    cancel_rx: rx,
                };
                run_agent_message(args).await;
            }
            disarm_cancel(&ctx.cancel_handle);
            // #899: keep busy-child snapshots fresh across auto-continue nudges
            // instead of frozen at the pre-turn snapshot until dispatch returns.
            super::uds_snapshots::refresh_busy_snapshots(ctx).await;
        }
    }
}
#[cfg(test)]
#[path = "uds_abort_steer_tests.rs"]
mod abort_steer_tests;
#[cfg(test)]
#[path = "uds_bounded_read_tests.rs"]
mod bounded_read_tests;
#[cfg(test)]
#[path = "uds_926_act_tests.rs"]
mod issue_926_act_tests;
#[cfg(test)]
#[path = "uds_parse_tests.rs"]
mod parse_tests;
#[cfg(test)]
#[path = "uds_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "uds_workflow_automation_tests.rs"]
mod workflow_automation_tests;
