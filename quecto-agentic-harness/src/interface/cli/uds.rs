use super::protocol::{AgentCommand, AgentEvent, StreamingBehavior};
#[cfg(test)]
use super::uds_cancel::CancelSlot;
use super::uds_cancel::{
    CancelHandle, EventSink, PromptOutcome, PromptRun, TurnControlHandle, arm_cancel,
    disarm_cancel, run_agent_message,
};
use super::uds_session::AgentSession;
#[cfg(test)]
use super::uds_session::{clear_conversation, resolve_rewind_target, rewind_to_message_index};
#[cfg(test)]
use super::uds_session::{
    compute_session_stats, compute_session_stats_with_usage, messages_tail_json,
};
#[cfg(test)]
use super::uds_socket::bind_secure_socket;
use super::uds_workflow_nudge::{workflow_nudge_message, workflow_progress_fingerprint};
use crate::application::agent_loop::AgentLoopImpl;
use crate::domain::message::Message;
#[cfg(test)]
use crate::domain::message::Role;
use crate::domain::session::SessionStore;
use crate::domain::workflow::WorkflowRunPersisted;
type ExtRegistry = std::sync::Arc<
    std::sync::Mutex<crate::infrastructure::extensions::registry::ExtensionRegistry>,
>;
pub use super::uds_lifecycle::{UdsLoopArgs, run_uds_loop};
pub(crate) use super::uds_lifecycle::{inject_system_prompt, remove_injected_system_prompt};
pub(crate) use super::uds_socket::reap_stale_sockets;
pub(super) const MAX_FRAME_PAYLOAD_BYTES: usize = quecto_line_io::PROTOCOL_LINE_CAP_BYTES;
pub(super) fn is_cancel_command(trimmed: &str) -> bool {
    is_abort_command(trimmed) || is_steer_command(trimmed)
}
/// Reader-side classification so the eager cancel and the abort/steer control
/// flag are set together, before the command is dispatched (#895/#896).
pub(super) fn is_abort_command(trimmed: &str) -> bool {
    command_type_is(trimmed, "abort")
}

pub(super) fn is_steer_command(trimmed: &str) -> bool {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return false;
    };
    v.get("type").and_then(|t| t.as_str()) == Some("steer")
        || (v.get("type").and_then(|t| t.as_str()) == Some("prompt")
            && v.get("streamingBehavior").and_then(|b| b.as_str()) == Some("steer"))
}

fn command_type_is(trimmed: &str, expected: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(trimmed)
        .ok()
        .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_owned))
        .as_deref()
        == Some(expected)
}

pub(super) enum LineResult {
    Command(AgentCommand),
    ParseError(String),
}

pub(super) fn parse_line(line: &str) -> LineResult {
    let line = line.trim();
    if line.is_empty() {
        return LineResult::ParseError(String::new());
    }
    match super::protocol::parse_command_line(line) {
        Ok(c) => LineResult::Command(c),
        Err(e) => LineResult::ParseError(e),
    }
}

pub(super) async fn run_command_loop(
    reader: Box<dyn tokio::io::AsyncRead + Send + Unpin>,
    ctx: &mut DispatchCtx<'_>,
) {
    use super::uds_reader::{ReaderMessage, spawn_reader_task};

    let cancel_for_reader = std::sync::Arc::clone(&ctx.cancel_handle);
    let control_for_reader = std::sync::Arc::clone(&ctx.turn_control);

    let (reader_task, mut rx) = spawn_reader_task(
        reader,
        cancel_for_reader,
        control_for_reader,
        ctx.wire_mode.clone(),
    );

    loop {
        let raw = match rx.recv().await {
            Some(Some(ReaderMessage::Message(l))) => l,
            Some(Some(ReaderMessage::ProtocolError(msg))) => {
                tracing::warn!("UDS protocol error: {msg}");
                let ev = AgentEvent::err(None, "protocol_error", msg);
                emit_event_to_broadcast_or_writer(ctx, &ev).await;
                continue;
            }
            _ => break,
        };

        match parse_line(&raw) {
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

pub(crate) struct DispatchCtx<'a> {
    pub wire_mode: super::uds_wire::ConnectionWireMode,
    pub base_dir: &'a std::path::Path,
    pub agent: &'a mut AgentLoopImpl,
    pub messages: &'a mut Vec<Message>,
    pub conversation_snapshot: super::uds_multi::ConversationSnapshot, // #828
    pub state_snapshot: super::uds_multi::StateSnapshot,               // #837
    pub execution_state: super::uds_execution_state::ExecutionStateHandle,
    pub session_stats_snapshot: super::uds_snapshots::SessionStatsSnapshot, // #880
    pub tool_catalogue_snapshot: super::uds_extensions::ToolCatalogueSnapshot, // #880
    pub busy: super::uds_multi::BusyFlag,                                   // #828
    pub session: &'a mut AgentSession,
    /// Direct writer for the single-client / test path. `None` on the
    /// multi-client server, which streams via `broadcast_tx` instead — so the
    /// server allocates no throwaway writer (#994).
    pub stdout: Option<&'a mut (dyn tokio::io::AsyncWrite + Send + Unpin)>,
    pub session_key: &'a mut String,
    pub session_store: &'a dyn SessionStore,
    pub ephemeral: bool,
    pub system_prompt: &'a str,
    pub cancel_handle: CancelHandle,
    /// Cross-task control flags for abort/steer vs workflow auto-continue
    /// (#895/#896). Set by the reader task, read at the idle boundary.
    pub turn_control: TurnControlHandle,
    pub broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    pub _ext_registry: Option<ExtRegistry>,
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
    /// The agent changed history that existed before its latest run.
    pub durable_prefix_dirty: bool,
}

impl<'a> DispatchCtx<'a> {
    /// The [`EventSink`] this context streams to: the broadcast channel on the
    /// multi-client server, otherwise the direct writer (#994).
    fn event_sink(&mut self) -> EventSink<'_> {
        make_event_sink(&self.broadcast_tx, &mut self.stdout, &self.wire_mode)
    }
}

/// Build an [`EventSink`] from a dispatch context's sink fields. Free function
/// (rather than only a `DispatchCtx` method) so callers that also need
/// disjoint borrows of other `DispatchCtx` fields (e.g. `run_agent_message`)
/// can split the borrow (#994).
fn make_event_sink<'s>(
    broadcast_tx: &Option<tokio::sync::broadcast::Sender<String>>,
    stdout: &'s mut Option<&mut (dyn tokio::io::AsyncWrite + Send + Unpin)>,
    wire_mode: &super::uds_wire::ConnectionWireMode,
) -> EventSink<'s> {
    if let Some(tx) = broadcast_tx {
        EventSink::Broadcast(tx.clone())
    } else {
        EventSink::writer_with_mode(
            stdout
                .as_deref_mut()
                .expect("dispatch context has neither broadcast_tx nor stdout"),
            wire_mode.clone(),
        )
    }
}

pub(super) async fn emit_event_to_broadcast_or_writer(
    ctx: &mut DispatchCtx<'_>,
    event: &AgentEvent,
) {
    ctx.event_sink().emit(event).await;
}

pub(super) async fn emit_ledger_advanced(
    ctx: &mut DispatchCtx<'_>,
    advance: super::uds_snapshots::LedgerAdvance,
) {
    ctx.event_sink().emit_ledger_advanced(advance).await;
}

/// Emit a command response, replacing an over-cap success payload with a small,
/// correlated error. Normal events reaching the cap are invariant violations;
/// responses can instead tell callers how to retry without silently hanging.
pub(super) async fn emit_response_or_frame_limit_error(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    command: &str,
    event: AgentEvent,
) {
    emit_response_or_frame_limit_error_with_message(ctx, id, command, event, "").await;
}

pub(super) async fn emit_response_or_frame_limit_error_with_message(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    command: &str,
    event: AgentEvent,
    oversized_message: &str,
) {
    // Serialize exactly once: the length check reuses the same line the sink
    // delivers (multi-MiB pages are not serialized twice).
    let line = event.to_json_line();
    if line.len() > super::protocol::EVENT_LINE_JSON_BUDGET {
        // "Request a smaller page" is only actionable for paged commands;
        // a single-message lookup has no smaller unit to retry with.
        let advice = if !oversized_message.is_empty() {
            oversized_message
        } else if command == "get_message" {
            "message exceeds the protocol frame limit and cannot be returned whole"
        } else {
            "response exceeds the protocol frame limit; request a smaller page"
        };
        let err = AgentEvent::err(id, command, advice);
        emit_event_to_broadcast_or_writer(ctx, &err).await;
    } else {
        ctx.event_sink().emit_serialized(line).await;
    }
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

#[path = "uds_dispatch.rs"]
mod uds_dispatch;
#[path = "uds_dispatch_forwarding.rs"]
mod uds_dispatch_forwarding;
#[path = "uds_dispatch_get_message_forward.rs"]
mod uds_dispatch_get_message_forward;
#[path = "uds_dispatch_query.rs"]
mod uds_dispatch_query;
#[path = "uds_dispatch_runtime.rs"]
mod uds_dispatch_runtime;
#[path = "uds_dispatch_session.rs"]
mod uds_dispatch_session;
#[path = "uds_dispatch_sync_forward.rs"]
mod uds_dispatch_sync_forward;
#[path = "uds_forward_response.rs"]
pub(super) mod uds_forward_response;
pub(crate) use uds_dispatch::dispatch_command;
#[cfg(test)]
use uds_dispatch_query::session_summary_to_json;
#[cfg(test)]
use uds_dispatch_runtime::resolve_set_model_target;

pub(super) struct PromptCommand {
    pub(super) id: Option<String>,
    pub(super) type_name: String,
    pub(super) message: String,
    pub(super) streaming_behavior: Option<StreamingBehavior>,
}

fn persisted_workflow_run(ctx: &DispatchCtx<'_>) -> Option<WorkflowRunPersisted> {
    ctx.workflow_state
        .as_ref()
        .and_then(|ws| ws.lock().ok().and_then(|engine| engine.persisted_run()))
}

async fn persist_user_prompt_before_run(
    ctx: &mut DispatchCtx<'_>,
    message: &Message,
) -> Result<(), crate::domain::error::DomainError> {
    if ctx.ephemeral || ctx.session_key.is_empty() {
        return Ok(());
    }
    let mut persisted_messages = ctx.messages.clone();
    remove_injected_system_prompt(&mut persisted_messages, ctx.system_prompt);
    persisted_messages.push(message.clone());
    let persisted_len = persisted_messages.len();
    let result = ctx
        .session_store
        .save_delta(
            ctx.session_key,
            &persisted_messages,
            ctx.last_persisted_message_index,
            persisted_workflow_run(ctx),
        )
        .await;
    if result.is_ok() {
        ctx.last_persisted_message_index = persisted_len;
    }
    result
}

pub(super) async fn handle_prompt(ctx: &mut DispatchCtx<'_>, cmd: PromptCommand) -> bool {
    let PromptCommand {
        id,
        type_name,
        message,
        streaming_behavior,
    } = cmd;
    if ctx.session.is_streaming() {
        match streaming_behavior {
            Some(StreamingBehavior::FollowUp) => {
                ctx.session.enqueue_pending(message);
                let ev = AgentEvent::ok(id.as_deref(), &type_name, None);
                emit_event_to_broadcast_or_writer(ctx, &ev).await;
            }
            Some(StreamingBehavior::Steer) => {
                ctx.session.prepend_pending(message);
                ctx.turn_control.clear_steer();
                let ev = AgentEvent::ok(id.as_deref(), "steer", None);
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
    let cancel_rx = match arm_cancel(&ctx.cancel_handle) {
        Some(rx) => rx,
        None if matches!(streaming_behavior, Some(StreamingBehavior::Steer)) => {
            ctx.turn_control.clear_steer();
            match arm_cancel(&ctx.cancel_handle) {
                Some(rx) => rx,
                None => {
                    emit_pre_cancelled(ctx).await;
                    drain_and_run_pending(ctx).await;
                    return false;
                }
            }
        }
        None => {
            emit_pre_cancelled(ctx).await; // Stale abort (#483).
            drain_and_run_pending(ctx).await;
            return false;
        }
    };
    let message = Message::user(message);
    if let Err(err) = persist_user_prompt_before_run(ctx, &message).await {
        tracing::warn!("failed to persist user prompt before turn: {err}");
    }
    // On success, persist_user_prompt_before_run already set last_persisted_message_index
    // to the durable (system-stripped) length. Do not overwrite with live len+1: live
    // messages may include an injected system prompt, which skews clean-delta appends
    // and freezes load()/resume history (#1322).
    let outcome = run_prompt_dispatch(ctx, message, cancel_rx).await;
    disarm_cancel(&ctx.cancel_handle);
    // #1072: persist_current_session drains the durable-prefix dirty latch after every outcome.
    if matches!(outcome, PromptOutcome::Success) {
        let ev = AgentEvent::ok(id.as_deref(), &type_name, None);
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
    }
    drain_pending_and_nudge(ctx).await;
    // Publish completed-turn state before any post-turn query queues behind the next command (#1104).
    super::uds_snapshots::refresh_busy_snapshots(ctx).await;
    // Persist after every turn so the conversation survives an ungraceful exit.
    if let Err(err) = uds_dispatch_session::persist_current_session(ctx).await {
        tracing::warn!("failed to persist session after turn: {err}");
    }
    false
}

/// Hard bound on nudged turns per idle drain, so a misbehaving model isn't
/// nudged forever. With the no-progress tolerance below, this cap is the only
/// termination guard against a model that keeps changing the fingerprint
/// (e.g. toggling a step) without ever finishing.
const MAX_WORKFLOW_NUDGES: usize = 128;

/// Drain pending messages, then inject core workflow nudges while progress is advancing (#562).
pub(super) async fn drain_pending_and_nudge(ctx: &mut DispatchCtx<'_>) {
    // #895: abort = full stop. A pending abort (set by the reader before this
    // command's handler runs) suppresses workflow auto-continue and discards
    // queued work, so the bound workflow does NOT resume at this idle boundary —
    // it stays stopped until re-driven by a fresh prompt. Abort beats any steer.
    if ctx.turn_control.take_abort() {
        ctx.turn_control.clear_steer();
        ctx.session.drain_pending();
        // #1082 review: an explicit abort is a requested stop, not a stall —
        // the reason keeps supervising monitors from raising a stall alert.
        emit_event_to_broadcast_or_writer(
            ctx,
            &AgentEvent::WorkflowIdle {
                reason: Some(super::protocol::WorkflowIdleReason::ExplicitAbort),
            },
        )
        .await;
        return;
    }

    drain_and_run_pending(ctx).await;

    // Consecutive no-progress nudged turns tolerated before giving up: two
    // corrective retries, the third consecutive no-progress turn breaks.
    // The nudge WORDING (standard and corrective) is owned by the domain
    // engine — this loop only decides which variant to send.
    const MAX_NO_PROGRESS_TURNS: usize = 3;

    let mut no_progress_turns = 0usize;
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
            emit_event_to_broadcast_or_writer(
                ctx,
                &AgentEvent::WorkflowIdle {
                    reason: Some(super::protocol::WorkflowIdleReason::ExplicitAbort),
                },
            )
            .await;
            return;
        }
        // #896: an explicit steer outranks the auto-continue nudge — yield so the
        // steered instruction is obeyed next instead of being overridden.
        if ctx.turn_control.is_steer_pending() {
            return;
        }
        let before = workflow_progress_fingerprint(ctx);
        let Some(nudge) = workflow_nudge_message(ctx) else {
            break;
        };
        let auto_continue = nudge.is_auto_continue();
        // A stalled previous nudged turn switches the auto-continue path to
        // its corrective wording: literal instruction-following models (e.g.
        // GPT-5.6) reply to the standard nudge with a bare status message and
        // no tool calls, so a verbatim repeat just re-elicits the same stall.
        //
        // The nudged turn runs ALONE inside the measured fingerprint window.
        // Messages that land in the pending queue while it streams (steer
        // follow-ups, buffered sub-agent notes) are drained AFTER the window
        // closes, so progress made by an unrelated turn is never attributed
        // to the nudge — it must not reset the no-progress streak or pick
        // the nudge wording.
        {
            let _busy = super::uds_multi::BusyGuard::new(&ctx.busy); // #828
            run_drained_message(
                ctx,
                Message::user(nudge.into_message(no_progress_turns > 0)),
            )
            .await;
        }
        let after = workflow_progress_fingerprint(ctx);
        drain_and_run_pending(ctx).await;
        if after == before {
            // The completion nudge is single-shot: it asks for a final report
            // and a stop, which never advances the fingerprint, so retrying
            // it would only elicit duplicate reports.
            if !auto_continue {
                break;
            }
            no_progress_turns += 1;
            if no_progress_turns >= MAX_NO_PROGRESS_TURNS {
                break;
            }
        } else {
            no_progress_turns = 0;
        }
    }
    let reason = super::uds_workflow_nudge::workflow_idle_reason(ctx);
    emit_event_to_broadcast_or_writer(
        ctx,
        &AgentEvent::WorkflowIdle {
            reason: Some(reason),
        },
    )
    .await;
}

async fn run_prompt_dispatch(
    ctx: &mut DispatchCtx<'_>,
    message: Message,
    cancel_rx: tokio::sync::oneshot::Receiver<()>,
) -> PromptOutcome {
    let _busy = super::uds_multi::BusyGuard::new(&ctx.busy); // #828: gates connect-time snapshot
    let mut sink = make_event_sink(&ctx.broadcast_tx, &mut ctx.stdout, &ctx.wire_mode);
    run_agent_message(PromptRun {
        agent: ctx.agent,
        messages: ctx.messages,
        conversation_snapshot: Some(ctx.conversation_snapshot.clone()),
        execution_state: Some(ctx.execution_state.clone()),
        session: ctx.session,
        sink: &mut sink,
        message,
        system_prompt: ctx.system_prompt,
        cancel_rx,
        notification_rx: &mut ctx.notification_rx,
        subagent_registry: &ctx.subagent_registry,
    })
    .await
}
async fn emit_pre_cancelled(ctx: &mut DispatchCtx<'_>) {
    emit_event_to_broadcast_or_writer(ctx, &AgentEvent::AgentStart).await;
    emit_event_to_broadcast_or_writer(
        ctx,
        &AgentEvent::AgentEnd {
            messages: vec![],
            message_refs: vec![],
        },
    )
    .await;
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
            run_drained_message(ctx, pending_msg.into_message()).await;
        }
    }
}

/// Run one drained or injected message through the agent: arm cancel, run,
/// disarm, refresh busy snapshots. A stale abort (#483) skips the run without
/// dropping the message's siblings — Fired is consumed, the next arm succeeds.
/// Callers own the busy flag (#828); this helper does not touch it, because
/// [`BusyGuard`](super::uds_multi::BusyGuard) is a plain set/clear flag and
/// nesting one per message would clear it while an outer scope is still busy.
async fn run_drained_message(ctx: &mut DispatchCtx<'_>, msg: Message) {
    let Some(rx) = arm_cancel(&ctx.cancel_handle) else {
        emit_pre_cancelled(ctx).await; // Stale abort (#483).
        return;
    };
    let mut sink = make_event_sink(&ctx.broadcast_tx, &mut ctx.stdout, &ctx.wire_mode);
    run_agent_message(PromptRun {
        agent: ctx.agent,
        messages: ctx.messages,
        conversation_snapshot: Some(ctx.conversation_snapshot.clone()),
        execution_state: Some(ctx.execution_state.clone()),
        session: ctx.session,
        sink: &mut sink,
        message: msg,
        system_prompt: ctx.system_prompt,
        cancel_rx: rx,
        notification_rx: &mut ctx.notification_rx,
        subagent_registry: &ctx.subagent_registry,
    })
    .await;
    disarm_cancel(&ctx.cancel_handle);
    // #1072: drained runs (steer follow-ups, workflow auto-continue,
    // coalesced sub-agent notes) can prune too. Their dirty latch is
    // sticky on the agent; `persist_current_session` drains it
    // centrally before choosing a persistence path.
    // #899: keep busy-child snapshots fresh across auto-continue nudges
    // instead of frozen at the pre-turn snapshot until dispatch returns.
    super::uds_snapshots::refresh_busy_snapshots(ctx).await;
}
#[cfg(test)]
#[path = "uds_abort_steer_tests.rs"]
mod abort_steer_tests;
#[cfg(test)]
#[path = "uds_bounded_read_tests.rs"]
mod bounded_read_tests;
#[cfg(test)]
#[path = "uds_dispatch_test_env.rs"]
mod dispatch_test_env;
#[cfg(test)]
#[path = "uds_effort_1067_tests.rs"]
mod effort_1067_tests;
#[cfg(test)]
#[path = "uds_926_act_tests.rs"]
mod issue_926_act_tests;
#[cfg(test)]
#[path = "uds_nudge_tolerance_tests.rs"]
mod nudge_tolerance_tests;
#[cfg(test)]
#[path = "uds_parse_tests.rs"]
mod parse_tests;
#[cfg(test)]
#[path = "uds_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "uds_workflow_automation_tests.rs"]
mod workflow_automation_tests;
