use super::protocol::{AgentCommand, AgentEvent, StreamingBehavior};
use super::uds_cancel::{
    CancelHandle, CancelSlot, PromptArgs, PromptMessageArgs, PromptOutcome, arm_cancel,
    disarm_cancel, fire_cancel, run_agent_message, run_agent_prompt,
};
use super::uds_multi::{MultiClientArgs, PromptArgsBroadcast, run_agent_prompt_broadcast};
use super::uds_session::{
    AgentSession, clear_conversation, compute_session_stats, message_to_json, messages_tail_json,
};
use crate::application::agent_loop::AgentLoopImpl;
use crate::domain::message::{Message, Role};
use crate::domain::session::{Session, SessionStore};
use crate::infrastructure::persistence::session_store::FileSessionStore;

pub use super::protocol::parse_command_line;

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
                messages,
                model,
                session_key,
                ephemeral,
                system_prompt,
                ext_registry,
                workflow_state,
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
                persist,
                notification_rx,
                subagent_registry,
                workflow_state,
                workflow_config,
                broadcast_tx,
            },
            listener,
            session_store,
        )
        .await
    }
}

struct SingleClientArgs {
    agent: AgentLoopImpl,
    messages: Vec<Message>,
    model: String,
    session_key: String,
    ephemeral: bool,
    system_prompt: String,
    ext_registry: Option<ExtRegistry>,
    workflow_state: Option<crate::interface::shared::WorkflowStateHandle>,
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
        mut session_key,
        ephemeral,
        system_prompt,
        ext_registry,
        workflow_state,
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

    run_command_loop(
        reader,
        &mut DispatchCtx {
            agent: &mut { agent },
            messages: &mut messages,
            session: &mut agent_session,
            stdout: &mut *writer,
            session_key: &mut session_key,
            session_store,
            ephemeral,
            system_prompt: &system_prompt,
            cancel_handle: std::sync::Arc::new(std::sync::Mutex::new(CancelSlot::Idle)),
            broadcast_tx: None,
            ext_registry,
            client_tool_registry: super::uds_ext_protocol::new_client_tool_registry(),
            current_client_id: 0,
            subagent_registry: None,
            notification_rx: None,
            workflow_state: workflow_state.clone(),
            workflow_config: None,
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
    trimmed.contains("\"type\":\"abort\"") || trimmed.contains("\"type\":\"steer\"")
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
    match parse_command_line(line) {
        Ok(c) => LineResult::Command(c),
        Err(e) => LineResult::ParseError(e),
    }
}

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

pub(super) struct DispatchCtx<'a> {
    pub agent: &'a mut AgentLoopImpl,
    pub messages: &'a mut Vec<Message>,
    pub session: &'a mut AgentSession,
    pub stdout: &'a mut (dyn tokio::io::AsyncWrite + Send + Unpin),
    pub session_key: &'a mut String,
    pub session_store: &'a dyn SessionStore,
    pub ephemeral: bool,
    pub system_prompt: &'a str,
    pub cancel_handle: CancelHandle,
    pub broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    pub ext_registry: Option<ExtRegistry>,
    pub client_tool_registry: super::uds_ext_protocol::ClientToolRegistry,
    pub current_client_id: u64,
    pub subagent_registry:
        Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
    pub notification_rx: Option<crate::infrastructure::tools::subagent_registry::NotificationRx>,
    pub workflow_state: Option<crate::interface::shared::WorkflowStateHandle>, // #562
    pub workflow_config: Option<crate::domain::workflow::WorkflowConfig>,      // #562
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

fn session_summary_to_json(summary: &crate::domain::session::SessionSummary) -> serde_json::Value {
    serde_json::json!({
        "key": summary.key,
        "name": summary.name,
        "messageCount": summary.message_count,
        "updatedUnixSecs": summary.updated_unix_secs,
    })
}

fn query_response_data(cmd: &AgentCommand, ctx: &DispatchCtx<'_>) -> Option<serde_json::Value> {
    match cmd {
        AgentCommand::GetState { .. } => {
            let workflow = ctx.workflow_state.as_ref().and_then(|ws| {
                ws.lock()
                    .ok()
                    .map(|engine| serde_json::to_value(engine.snapshot(true)).unwrap_or_default())
            });
            let state = ctx.session.state_snapshot(ctx.messages.len(), workflow);
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
        AgentCommand::GetSubagents { .. } => {
            let list = super::protocol::build_subagent_info_list(&ctx.subagent_registry);
            Some(serde_json::json!({ "subagents": list }))
        }
        AgentCommand::ReloadExtensions { .. } => {
            // Handled in dispatch_command (needs async I/O + broadcast).
            None
        }
        _ => None,
    }
}

/// Returns `Some(bool)` if handled, `None` to fall through to the main match.
async fn dispatch_fieldless_command(cmd: &AgentCommand, ctx: &mut DispatchCtx<'_>) -> Option<bool> {
    let id = cmd.id();
    let tn = cmd.type_name();
    if matches!(cmd, AgentCommand::ListSessions { .. }) {
        let event = match ctx.session_store.list().await {
            Ok(sessions) => AgentEvent::ok(
                id,
                tn,
                Some(serde_json::json!({
                    "sessions": sessions
                        .iter()
                        .filter(|session| session.key.starts_with("cli:"))
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

pub(super) async fn dispatch_command(cmd: AgentCommand, ctx: &mut DispatchCtx<'_>) -> bool {
    // Fast path: queries + clear_history (defers id/type_name clones).
    if let Some(result) = dispatch_fieldless_command(&cmd, ctx).await {
        return result;
    }

    let id = cmd.id().map(str::to_owned);
    let type_name = cmd.type_name().to_owned();

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
        AgentCommand::ResumeSession { session, .. } => {
            handle_resume_session(ctx, id.as_deref(), &type_name, session).await
        }
        AgentCommand::ReloadExtensions { .. }
        | AgentCommand::RegisterTools { .. }
        | AgentCommand::UnregisterTools { .. }
        | AgentCommand::ToolResult { .. } => {
            dispatch_ext_command(cmd, ctx, id.as_deref(), &type_name).await
        }
        // Exhaustive: variants handled by dispatch_fieldless_command above.
        AgentCommand::ClearHistory { .. }
        | AgentCommand::GetExtensions { .. }
        | AgentCommand::GetSubagents { .. }
        | AgentCommand::GetState { .. }
        | AgentCommand::GetMessages { .. }
        | AgentCommand::GetMessagesTail { .. }
        | AgentCommand::GetSessionStats { .. }
        | AgentCommand::ListSessions { .. } => {
            tracing::error!(command = %type_name, "fieldless variant reached dispatch fallback");
            let ev = AgentEvent::err(id.as_deref(), &type_name, "internal: unhandled command");
            emit_event_to_broadcast_or_writer(ctx, &ev).await;
            false
        }
    }
}

async fn persist_current_session(
    ctx: &mut DispatchCtx<'_>,
) -> Result<(), crate::domain::error::DomainError> {
    if ctx.ephemeral || ctx.session_key.is_empty() {
        return Ok(());
    }
    remove_injected_system_prompt(ctx.messages, ctx.system_prompt);
    let session = Session {
        key: ctx.session_key.clone(),
        messages: ctx.messages.clone(),
        workflow_run: ctx
            .workflow_state
            .as_ref()
            .and_then(|ws| ws.lock().ok().and_then(|engine| engine.persisted_run())),
    };
    inject_system_prompt(ctx.messages, ctx.system_prompt);
    ctx.session_store.save(&session).await
}

async fn handle_resume_session(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    type_name: &str,
    session: String,
) -> bool {
    if ctx.session.is_streaming() {
        let ev = AgentEvent::err(
            id,
            type_name,
            "cannot resume a session while agent is running",
        );
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
        return false;
    }
    if ctx.ephemeral {
        let ev = AgentEvent::err(id, type_name, "cannot resume sessions in ephemeral mode");
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
        return false;
    }
    let name = session.trim();
    if !crate::interface::cli::is_valid_session_name(name) {
        let ev = AgentEvent::err(
            id,
            type_name,
            "session name must contain only alphanumeric, '-', or '_'",
        );
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
        return false;
    }
    let new_key = Session::build_key("cli", name);
    if let Err(err) = persist_current_session(ctx).await {
        let ev = AgentEvent::err(
            id,
            type_name,
            format!("failed to save current session: {err}"),
        );
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
        return false;
    }
    let loaded = match ctx.session_store.load(&new_key).await {
        Ok(Some(session)) => session,
        Ok(None) => {
            let ev = AgentEvent::err(id, type_name, format!("session not found: {name}"));
            emit_event_to_broadcast_or_writer(ctx, &ev).await;
            return false;
        }
        Err(err) => {
            let ev = AgentEvent::err(id, type_name, format!("failed to load session: {err}"));
            emit_event_to_broadcast_or_writer(ctx, &ev).await;
            return false;
        }
    };
    *ctx.session_key = new_key.clone();
    ctx.session.set_session_key(new_key.clone());
    ctx.session.drain_pending();
    *ctx.messages = loaded.messages;
    inject_system_prompt(ctx.messages, ctx.system_prompt);
    let ev = AgentEvent::ok(
        id,
        type_name,
        Some(serde_json::json!({
            "session": name,
            "sessionKey": new_key,
            "messageCount": ctx.messages.len(),
        })),
    );
    emit_event_to_broadcast_or_writer(ctx, &ev).await;
    false
}

async fn handle_steer(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    type_name: &str,
    message: String,
) -> bool {
    if ctx.session.is_streaming() {
        // Reader task already fires cancel eagerly — do NOT fire again here (#512).
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
    // Reader task already fires cancel eagerly — do NOT fire again here (#512).
    let ev = AgentEvent::ok(id, type_name, None);
    emit_event_to_broadcast_or_writer(ctx, &ev).await;
    false
}
async fn handle_clear_history(ctx: &mut DispatchCtx<'_>, id: Option<&str>, tn: &str) -> bool {
    if ctx.session.is_streaming() {
        let ev = AgentEvent::err(id, tn, "cannot clear history while agent is running");
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
        return false;
    }
    clear_conversation(ctx.messages);
    ctx.session.drain_pending();
    // Also clear spill store so stale context isn't re-injected (#412).
    if let Some(spill) = ctx.agent.spill_store() {
        if let Err(e) = spill.clear(ctx.session_key).await {
            tracing::warn!("clear_history: failed to clear spill store: {e}");
        }
    }
    let ev = AgentEvent::ok(id, tn, None);
    emit_event_to_broadcast_or_writer(ctx, &ev).await;
    false
}
use super::uds_extensions::{build_extension_list, handle_reload_extensions};
async fn dispatch_ext_command(
    cmd: AgentCommand,
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    tn: &str,
) -> bool {
    match cmd {
        AgentCommand::ReloadExtensions { .. } => handle_reload_extensions(ctx, id, tn).await,
        AgentCommand::RegisterTools { tools, .. } => {
            super::uds_ext_protocol::dispatch_register_tools(ctx, id, &tools).await
        }
        AgentCommand::UnregisterTools { tools, .. } => {
            super::uds_ext_protocol::dispatch_unregister_tools(ctx, id, &tools).await
        }
        AgentCommand::ToolResult {
            tool_call_id,
            content,
            is_error,
        } => super::uds_ext_protocol::dispatch_tool_result(ctx, &tool_call_id, &content, is_error),
        _ => {}
    };
    false
}
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
            }
            None => {
                let msg = "agent is running; provide streamingBehavior";
                let ev = AgentEvent::err(id.as_deref(), &type_name, msg);
                emit_event_to_broadcast_or_writer(ctx, &ev).await;
            }
        }
        return false;
    }
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
    false
}

/// Drain pending messages, then inject a workflow nudge if applicable (#562).
async fn drain_pending_and_nudge(ctx: &mut DispatchCtx<'_>) {
    drain_and_run_pending(ctx).await;
    let nudged = match (&ctx.workflow_state, &ctx.workflow_config) {
        (Some(ws), Some(wc)) if wc.auto_continue || wc.completion_nudge => {
            let Ok(s) = ws.lock() else { return };
            let n = (wc.auto_continue)
                .then(|| s.auto_continue_nudge())
                .flatten()
                .or_else(|| {
                    (wc.completion_nudge)
                        .then(|| s.completion_nudge())
                        .flatten()
                });
            if let Some(m) = n {
                ctx.session.enqueue_pending(m);
                true
            } else {
                false
            }
        }
        _ => false,
    };
    if nudged {
        drain_and_run_pending(ctx).await;
    }
}

async fn run_prompt_dispatch(
    ctx: &mut DispatchCtx<'_>,
    message: String,
    cancel_rx: tokio::sync::oneshot::Receiver<()>,
) -> PromptOutcome {
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
    loop {
        let pending = ctx.session.drain_pending();
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
        }
    }
}
#[cfg(test)]
#[path = "uds_tests.rs"]
mod tests;
