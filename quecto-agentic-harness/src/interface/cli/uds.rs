use super::protocol::{AgentCommand, AgentEvent, StreamingBehavior};
#[cfg(test)]
use super::uds_cancel::CancelSlot;
use super::uds_cancel::{
    CancelHandle, EventSink, PromptOutcome, PromptRun, TurnControlHandle, arm_cancel,
    disarm_cancel, run_agent_message,
};
use super::uds_query::query_response_data;
use super::uds_session::{
    AgentSession, clear_conversation, resolve_rewind_target, rewind_to_message_index,
};
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
use crate::domain::session::{Session, SessionStore};
use crate::infrastructure::model_registry::ModelRegistry;

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
    trimmed.contains("\"type\":\"abort\"")
}

pub(super) fn is_steer_command(trimmed: &str) -> bool {
    trimmed.contains("\"type\":\"steer\"")
}

pub(super) enum LineResult {
    Command(AgentCommand),
    ParseError(String),
}

// NOTE: a `LineTooLong` variant used to live here, produced by a post-hoc
// `line.len() > MAX_FRAME_PAYLOAD_BYTES` check. Since #1003 both reader loops enforce
// the cap *while reading* (`quecto_line_io::read_bounded_line`) and surface
// oversized lines before `parse_line` is ever called, so the variant was
// unreachable and has been removed.
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
    pub session_stats_snapshot: super::uds_snapshots::SessionStatsSnapshot, // #880
    pub extension_snapshot: super::uds_extensions::ExtensionSnapshot,  // #880
    pub busy: super::uds_multi::BusyFlag,                              // #828
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

/// Emit a command response, replacing an over-cap success payload with a small,
/// correlated error. Normal events reaching the cap are invariant violations;
/// responses can instead tell callers how to retry without silently hanging.
async fn emit_response_or_frame_limit_error(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    command: &str,
    event: AgentEvent,
) {
    // Serialize exactly once: the length check reuses the same line the sink
    // delivers (multi-MiB pages are not serialized twice).
    let line = event.to_json_line();
    if line.len() > super::protocol::EVENT_LINE_JSON_BUDGET {
        // "Request a smaller page" is only actionable for paged commands;
        // a single-message lookup has no smaller unit to retry with.
        let advice = if command == "get_message" {
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
    // #935/#1044: re-derive the per-model output cap AND context window so a
    // model switch re-clamps subsequent turns and the pruning budget; one
    // registry load feeds both, and set_model takes them atomically so model,
    // cap, and window can never diverge.
    let (cap, window) = ModelRegistry::model_limits_from_base_dir(ctx.base_dir, &resolved_model);
    ctx.agent.set_model(resolved_model.clone(), cap, window);
    ctx.session.set_model(resolved_model);
    // Every model switch resets the session effort to `low` (#1067): a level
    // chosen for one provider (e.g. OpenAI `xhigh`) must not silently carry
    // into another provider's vocabulary, where it would be clamped on the
    // wire while the UI still displays the stale level. Explicit `low` is
    // predictable and cost-safe; the user re-raises effort via set_effort.
    ctx.agent
        .set_effort(crate::domain::provider::EffortLevel::Low);
    tracing::debug!(new_model = %ctx.session.model(), "UDS: model switched; effort reset to low");
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
    // #1060 review 1a: resolve get_message against the id-addressable ledger
    // (full copies) before the live conversation, so a ref pruned/collapsed
    // from `ctx.messages` still resolves to full content. The ledger wins over
    // a possibly-collapsed live entry.
    if let AgentCommand::GetMessage {
        message_id,
        offset,
        limit,
        ..
    } = cmd
    {
        let resolved =
            super::uds_snapshots::resolve_get_message(&ctx.conversation_snapshot, message_id)
                .await
                .map(|msg| {
                    super::uds_session::message_to_json_range_for_response(
                        &msg, *offset, *limit, id,
                    )
                });
        let ev = match resolved.or_else(|| {
            super::uds_query::get_message_response_data(message_id, *offset, *limit, id, ctx)
        }) {
            Some(data) => AgentEvent::ok(id, tn, Some(data)),
            None => AgentEvent::err(id, tn, format!("message not found: {message_id}")),
        };
        emit_response_or_frame_limit_error(ctx, id, tn, ev).await;
        return Some(false);
    }
    // A supplied paging cursor is a stable message id. Treat a stale/unknown
    // id as an error instead of silently restarting at the newest page, which a
    // client would otherwise prepend and duplicate as "older" history.
    if let AgentCommand::GetMessages {
        before: Some(cursor),
        ..
    } = cmd
        && super::uds_session::position_by_wire_id(ctx.messages, cursor).is_none()
    {
        let ev = AgentEvent::err(id, tn, format!("history cursor not found: {cursor}"));
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
        return Some(false);
    }
    if let Some(data) = query_response_data(cmd, ctx) {
        emit_response_or_frame_limit_error(ctx, id, tn, AgentEvent::ok(id, tn, Some(data))).await;
        return Some(false);
    }
    if matches!(cmd, AgentCommand::ClearHistory { .. }) {
        return Some(handle_clear_history(ctx, id, tn).await);
    }
    None
}

#[path = "uds_dispatch.rs"]
mod uds_dispatch;
#[path = "uds_dispatch_get_message_forward.rs"]
mod uds_dispatch_get_message_forward;
#[path = "uds_forward_response.rs"]
mod uds_forward_response;
pub(crate) use uds_dispatch::dispatch_command;
use uds_dispatch::handle_clear_history;

struct PromptCommand {
    id: Option<String>,
    type_name: String,
    message: String,
    streaming_behavior: Option<StreamingBehavior>,
}

async fn handle_prompt(ctx: &mut DispatchCtx<'_>, cmd: PromptCommand) -> bool {
    // A prompt clears any stale substring-detected steer gate (#896 AC3).
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
    // #1072: the agent's durable-prefix dirty latch is drained centrally by
    // `persist_current_session` (below), after EVERY outcome — pruning may
    // have mutated history before an Error or a Cancelled ending too.
    if matches!(outcome, PromptOutcome::Success) {
        let ev = AgentEvent::ok(id.as_deref(), &type_name, None);
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
    }
    drain_pending_and_nudge(ctx).await;
    // Publish the completed-turn state before any post-turn query waits behind
    // the next command in the dispatch loop. This closes the boundary where a
    // just-idle child could still hand a newly connecting inspector the previous
    // busy snapshot (#1104).
    super::uds_snapshots::refresh_busy_snapshots(ctx).await;
    // Persist after every turn so the conversation survives an ungraceful exit.
    if let Err(err) = uds_dispatch::persist_current_session(ctx).await {
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
    message: String,
    cancel_rx: tokio::sync::oneshot::Receiver<()>,
) -> PromptOutcome {
    let _busy = super::uds_multi::BusyGuard::new(&ctx.busy); // #828: gates connect-time snapshot
    let mut sink = make_event_sink(&ctx.broadcast_tx, &mut ctx.stdout, &ctx.wire_mode);
    run_agent_message(PromptRun {
        agent: ctx.agent,
        messages: ctx.messages,
        conversation_snapshot: Some(ctx.conversation_snapshot.clone()),
        session: ctx.session,
        sink: &mut sink,
        message: Message::user(message),
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
        session: ctx.session,
        sink: &mut sink,
        message: msg,
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
