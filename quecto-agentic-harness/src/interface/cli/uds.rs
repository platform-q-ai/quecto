use super::protocol::{AgentCommand, AgentEvent, StreamingBehavior};
#[cfg(test)]
use super::uds_cancel::CancelSlot;
use super::uds_cancel::{
    CancelHandle, EventSink, PromptOutcome, PromptRun, TurnControlHandle, disarm_cancel,
    run_agent_message,
};
use super::uds_session::AgentSession;
#[cfg(test)]
use super::uds_session::{
    compute_session_stats, compute_session_stats_with_usage, messages_page_json,
};
#[cfg(test)]
use super::uds_socket::bind_secure_socket;
use super::uds_workflow_nudge::{
    has_active_workflow_descendant, workflow_nudge_message, workflow_progress_fingerprint,
};
use crate::application::agent_loop::AgentLoopImpl;
use crate::application::sessions::dto::SaveTrigger;
use crate::application::subagents::use_cases::TerminateAllDelegatedAgents;
use crate::domain::message::Message;
use crate::interface::cli::uds_discovery_handles::SessionDiscoveryHandles;
use futures::FutureExt;
type ExtRegistry = std::sync::Arc<
    std::sync::Mutex<crate::infrastructure::extensions::registry::ExtensionRegistry>,
>;
pub(crate) use super::uds_lifecycle::inject_system_prompt;
#[cfg(test)]
pub(crate) use super::uds_lifecycle::remove_injected_system_prompt;
pub use super::uds_lifecycle::{UdsLoopArgs, run_uds_loop};
// `pub` (not `pub(crate)`) solely so the out-of-crate BDD suite can drive the
// real reaper; hidden because it is not a supported library API.
#[doc(hidden)]
pub use super::uds_socket::reap_stale_sockets;
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
    matches!(
        serde_json::from_str::<AgentCommand>(trimmed),
        Ok(AgentCommand::Steer { .. }
            | AgentCommand::Prompt {
                streaming_behavior: Some(StreamingBehavior::Steer),
                ..
            })
    )
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
    pub agent: &'a mut AgentLoopImpl,
    pub messages: &'a mut Vec<Message>,
    /// The active session and its read use cases (#1971): the one
    /// conversation read model every transport serves from.
    pub sessions: super::uds_session_handles::SessionReadHandles,
    pub state_snapshot: super::uds_multi::StateSnapshot, // #837
    pub execution_state: super::uds_execution_state::ExecutionStateHandle,
    pub session_stats_snapshot: super::uds_snapshots::SessionStatsSnapshot, // #880
    pub tool_catalogue_snapshot: super::uds_extensions::ToolCatalogueSnapshot, // #880
    pub busy: super::uds_multi::BusyFlag,                                   // #828
    pub session: &'a mut AgentSession,
    /// Direct writer for the single-client / test path. `None` on the
    /// multi-client server, which streams via `broadcast_tx` instead — so the
    /// server allocates no throwaway writer (#994).
    pub stdout: Option<&'a mut (dyn tokio::io::AsyncWrite + Send + Unpin)>,
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
    /// Fleet teardown (#1938) of delete-all and session transitions.
    pub fleet_teardown: Option<FleetTeardown>,
    /// Discovery (#1861, #2010): answers `list_sessions` and `search_session_metadata`.
    pub discovery: SessionDiscoveryHandles,
    /// Save current session (#1860, #1972): the one transaction every
    /// persistence trigger of this loop requests; it owns the watermark,
    /// the dirty latch and the killing-exit state on the active session.
    pub save_session: SaveSessionHandle,
    /// Clear (#1864) and rewind (#1865) the conversation (#1975): the two
    /// history-replacing transactions this loop requests once admitted.
    pub rewrite: super::uds_session_handles::ConversationRewriteHandles,
    /// Start a fresh conversation (#1862, #1976) or resume a saved one (#1863,
    /// #1977): the two session transitions this loop requests once admitted.
    pub switch: super::uds_session_handles::SessionSwitchHandles,
    /// The catalogue handles (#1845, #1848): `list_models` and effort changes.
    pub catalogue: super::catalogue_handles::CatalogueHandles,
}
type FleetTeardown = std::sync::Arc<TerminateAllDelegatedAgents>;
type SaveSessionHandle = std::sync::Arc<crate::application::sessions::use_cases::SaveSession>;

impl<'a> DispatchCtx<'a> {
    /// The [`EventSink`] this context streams to: the broadcast channel on the
    /// multi-client server, otherwise the direct writer (#994).
    fn event_sink(&mut self) -> EventSink<'_> {
        make_event_sink(&self.broadcast_tx, &mut self.stdout, &self.wire_mode)
    }
}

/// Build an [`EventSink`] from a dispatch context's sink fields. A free
/// function so callers that also need disjoint borrows of other `DispatchCtx`
/// fields (e.g. `run_agent_message`) can split the borrow (#994).
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
    advance: crate::application::sessions::conversation_ledger::LedgerAdvance,
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
pub(crate) mod uds_dispatch_session;
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

/// Persist the user prompt about to run so it survives an ungraceful exit
/// mid-turn; the transaction stamps its durable ordinal (#1322).
async fn persist_user_prompt_before_run(
    ctx: &mut DispatchCtx<'_>,
    message: &mut Message,
) -> Result<(), crate::application::sessions::dto::SaveSessionError> {
    ctx.save_session
        .save_with_pending_prompt(ctx.messages, message)
        .await
        .map(|_| ())
}

pub(super) async fn handle_prompt(ctx: &mut DispatchCtx<'_>, cmd: PromptCommand) -> bool {
    let PromptCommand {
        id,
        type_name,
        message,
        streaming_behavior,
    } = cmd;
    if ctx.session.is_streaming() {
        prompt_admission::handle_busy_prompt(
            ctx,
            id.as_deref(),
            &type_name,
            message,
            streaming_behavior,
        )
        .await;
        return false;
    }
    if ctx.turn_control.is_steer_pending()
        && !matches!(streaming_behavior, Some(StreamingBehavior::Steer))
    {
        pending::queue_prompt(ctx, id.as_deref(), &type_name, message, false).await;
        return false;
    }
    ctx.session.resume_automatic_turns();
    // #1721: a failure during this turn is dated at the generation known now.
    ctx.session
        .observe_control_generation(ctx.turn_control.control_generation());
    // Reload runtime configuration (#1849): the pull-based poll before every
    // prompt (ADR-0002), rebuilt off the runtime; a failure keeps last-good.
    super::uds_dispatch_reload::poll_reload(ctx).await;
    let cancel_rx = arm_prompt_cancel(
        ctx,
        matches!(streaming_behavior, Some(StreamingBehavior::Steer)),
    )
    .await;
    let Some(cancel_rx) = cancel_rx else {
        ctx.session.record_control(
            id.as_deref(),
            &type_name,
            super::protocol::ControlStatus::Cancelled,
        );
        emit_pre_cancelled(ctx).await; // Stale abort (#483).
        drain_and_run_pending(ctx).await;
        super::uds_swarm_control::date_provider_suspension(ctx).await;
        return false;
    };
    let mut message = Message::user(message);
    if let Err(err) = persist_user_prompt_before_run(ctx, &mut message).await {
        tracing::warn!("failed to persist user prompt before turn: {err}");
    }
    // On success the save transaction already set the persisted watermark to
    // the durable (system-stripped) length. Never overwrite it with live
    // len+1: live messages may include an injected system prompt, which skews
    // clean-delta appends and freezes load()/resume history (#1322).
    ctx.session.record_control(
        id.as_deref(),
        &type_name,
        super::protocol::ControlStatus::Started,
    );
    let outcome = run_prompt_dispatch(ctx, message, cancel_rx).await;
    ctx.session
        .record_control(id.as_deref(), &type_name, control_status(&outcome));
    disarm_cancel(&ctx.cancel_handle);
    // #1072: the save transaction drains the durable-prefix dirty latch after every outcome.
    if matches!(outcome, PromptOutcome::Success) {
        let ev = AgentEvent::ok(id.as_deref(), &type_name, None);
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
    }
    drain_pending_and_nudge(ctx).await;
    // Publish completed-turn state before any post-turn query queues behind the next command (#1104).
    super::uds_snapshots::refresh_busy_snapshots(ctx).await;
    // Persist after every turn so the conversation survives an ungraceful exit.
    if let Err(err) = ctx
        .save_session
        .save(ctx.messages, SaveTrigger::Routine)
        .await
    {
        tracing::warn!("failed to persist session after turn: {err}");
    }
    false
}

#[path = "uds_prompt_admission.rs"]
mod prompt_admission;
use prompt_admission::{arm_prompt_cancel, control_status, emit_pre_cancelled};

/// Hard bound on nudged turns per idle drain, so a misbehaving model isn't
/// nudged forever. With the no-progress tolerance below, this cap is the only
/// termination guard against a model that keeps changing the fingerprint
/// (e.g. toggling a step) without ever finishing.
const MAX_WORKFLOW_NUDGES: usize = 128;

/// Drain pending messages, then inject core workflow nudges while progress is advancing (#562).
#[cfg(test)]
static BEFORE_WORKFLOW_NUDGE_INJECTION_TEST_HOOK: std::sync::Mutex<Option<Box<dyn Fn() + Send>>> =
    std::sync::Mutex::new(None);
#[cfg(test)]
static BEFORE_GUARDED_TURN_ADMISSION_TEST_HOOK: std::sync::Mutex<Option<Box<dyn Fn() + Send>>> =
    std::sync::Mutex::new(None);

#[cfg(test)]
fn run_before_workflow_nudge_injection_test_hook() {
    if let Some(hook) = BEFORE_WORKFLOW_NUDGE_INJECTION_TEST_HOOK
        .lock()
        .unwrap()
        .take()
    {
        hook();
    }
}

#[cfg(test)]
pub(super) fn set_before_workflow_nudge_injection_test_hook(hook: Box<dyn Fn() + Send>) {
    *BEFORE_WORKFLOW_NUDGE_INJECTION_TEST_HOOK.lock().unwrap() = Some(hook);
}

#[cfg(test)]
fn run_before_guarded_turn_admission_test_hook() {
    if let Some(hook) = BEFORE_GUARDED_TURN_ADMISSION_TEST_HOOK
        .lock()
        .unwrap()
        .take()
    {
        hook();
    }
}

#[cfg(test)]
pub(super) fn set_before_guarded_turn_admission_test_hook(hook: Box<dyn Fn() + Send>) {
    *BEFORE_GUARDED_TURN_ADMISSION_TEST_HOOK.lock().unwrap() = Some(hook);
}

pub(super) async fn drain_pending_and_nudge(ctx: &mut DispatchCtx<'_>) {
    drain_pending_and_nudge_turns(ctx).await;
    // #1721: whichever turn failed (prompt, drained or nudged), date its
    // provider-failure suspension by the generation current after it.
    super::uds_swarm_control::date_provider_suspension(ctx).await;
}

async fn drain_pending_and_nudge_turns(ctx: &mut DispatchCtx<'_>) {
    // #895: abort = full stop. A pending abort (set by the reader before this
    // command's handler runs) suppresses workflow auto-continue and discards
    // queued work, so the bound workflow does NOT resume at this idle boundary —
    // it stays stopped until re-driven by a fresh prompt. Abort beats any steer.
    if ctx.turn_control.take_abort() {
        ctx.turn_control.clear_steer();
        ctx.session.discard_pending();
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
    if !ctx.session.automatic_turns_allowed {
        return;
    }

    // Consecutive no-progress nudged turns tolerated before giving up: two
    // corrective retries, the third consecutive no-progress turn breaks.
    // The nudge WORDING (standard and corrective) is owned by the domain
    // engine — this loop only decides which variant to send.
    const MAX_NO_PROGRESS_TURNS: usize = 3;

    let mut no_progress_turns = 0usize;
    for _ in 0..MAX_WORKFLOW_NUDGES {
        if !ctx.session.automatic_turns_allowed {
            break;
        }
        // #930: an abort that lands WHILE this auto-continue loop is mid-flight is
        // a full stop, exactly like one at the idle-drain entry above — discard
        // queued work and stop nudging. The entry guard only catches an abort that
        // arrived before the drain; without re-checking here, a workflow that was
        // already auto-continuing kept advancing past the abort (e.g. 5/17 → 9/17)
        // because the loop only broke on a pending steer or a no-progress turn.
        if ctx.turn_control.take_abort() {
            ctx.turn_control.clear_steer();
            ctx.session.discard_pending();
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
        let Some(nudge) = workflow_nudge_message(ctx).await else {
            break;
        };
        let auto_continue = nudge.is_auto_continue();
        // The nudge-selection descendant check is only a snapshot. Re-check the
        // same harness-level identity/descendant predicate immediately before
        // injection so a child that becomes starting/running between selection
        // and model execution cancels this auto turn instead of racing it.
        #[cfg(test)]
        run_before_workflow_nudge_injection_test_hook();
        if has_active_workflow_descendant(ctx).await {
            break;
        }
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
            run_drained_message_guarded(
                ctx,
                Message::user(nudge.into_message(no_progress_turns > 0)),
                TurnAdmissionGuard::NoActiveWorkflowDescendant,
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
        active_session: Some(ctx.sessions.active_session.clone()),
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
#[path = "uds_pending.rs"]
mod pending;
use pending::drain_and_run_pending;

/// Run one drained or injected message through the agent: arm cancel, run,
/// disarm, refresh busy snapshots. A stale abort (#483) skips the run without
/// dropping the message's siblings — Fired is consumed, the next arm succeeds.
/// Callers own the busy flag (#828); this helper does not touch it, because
/// [`BusyGuard`](super::uds_multi::BusyGuard) is a plain set/clear flag and
/// nesting one per message would clear it while an outer scope is still busy.
#[derive(Clone, Copy)]
enum TurnAdmissionGuard {
    None,
    NoActiveWorkflowDescendant,
}

async fn run_drained_message(ctx: &mut DispatchCtx<'_>, msg: Message) -> PromptOutcome {
    run_drained_message_guarded(ctx, msg, TurnAdmissionGuard::None).await
}

async fn run_drained_message_guarded(
    ctx: &mut DispatchCtx<'_>,
    msg: Message,
    guard: TurnAdmissionGuard,
) -> PromptOutcome {
    #[cfg(test)]
    run_before_guarded_turn_admission_test_hook();
    let Some(rx) = super::uds_cancel::arm_swarm_cancel(&ctx.cancel_handle, &ctx.turn_control).await
    else {
        emit_pre_cancelled(ctx).await; // Stale abort (#483).
        return PromptOutcome::Cancelled;
    };
    let session_key = ctx.sessions.current_session_key().await;
    let outcome = {
        let mut sink = make_event_sink(&ctx.broadcast_tx, &mut ctx.stdout, &ctx.wire_mode);
        let run = run_agent_message(PromptRun {
            agent: ctx.agent,
            messages: ctx.messages,
            active_session: Some(ctx.sessions.active_session.clone()),
            execution_state: Some(ctx.execution_state.clone()),
            session: ctx.session,
            sink: &mut sink,
            message: msg,
            system_prompt: ctx.system_prompt,
            cancel_rx: rx,
            notification_rx: &mut ctx.notification_rx,
            subagent_registry: &ctx.subagent_registry,
        });
        tokio::pin!(run);

        // Polling the future once admits the turn: all synchronous setup before its
        // first await happens while the registry is locked, so child registration
        // cannot slip between the final descendant check and turn start. The lock is
        // released before awaiting the rest of the model/tool execution; a nudge may
        // therefore spawn children normally without deadlocking the registry.
        let admitted = if matches!(guard, TurnAdmissionGuard::NoActiveWorkflowDescendant) {
            let identity =
                crate::infrastructure::tools::subagent_identity::parent_identity_from_session_key(
                    session_key.as_str(),
                );
            if let (Some(identity), Some(registry)) = (identity, &ctx.subagent_registry) {
                let entries = registry.lock().unwrap_or_else(|e| e.into_inner());
                if crate::infrastructure::tools::subagent_registry::has_active_descendant_for_agent_locked(
                &entries, identity,
            ) {
                drop(entries);
                disarm_cancel(&ctx.cancel_handle);
                return PromptOutcome::Cancelled;
            }
                let result = run.as_mut().now_or_never();
                drop(entries);
                result
            } else {
                run.as_mut().now_or_never()
            }
        } else {
            run.as_mut().now_or_never()
        };
        match admitted {
            Some(outcome) => outcome,
            None => run.as_mut().await,
        }
    };
    disarm_cancel(&ctx.cancel_handle);
    // #1072: drained runs (steer follow-ups, workflow auto-continue,
    // coalesced sub-agent notes) can prune too. Their dirty latch is
    // sticky on the agent; the save transaction drains it centrally
    // before choosing a persistence path.
    // #899: keep busy-child snapshots fresh across auto-continue nudges
    // instead of frozen at the pre-turn snapshot until dispatch returns.
    super::uds_snapshots::refresh_busy_snapshots(ctx).await;
    outcome
}
#[cfg(test)]
#[path = "uds_abort_steer_tests.rs"]
mod abort_steer_tests;
#[cfg(test)]
#[path = "uds_bounded_read_tests.rs"]
mod bounded_read_tests;
#[cfg(test)]
#[path = "uds_dispatch_session_roster_tests.rs"]
pub(crate) mod dispatch_session_roster_tests;
#[cfg(test)]
#[path = "uds_dispatch_test_env.rs"]
mod dispatch_test_env;
#[cfg(test)]
#[path = "uds_effort_1067_tests.rs"]
mod effort_1067_tests;
#[cfg(test)]
#[path = "uds_issue1567_tests.rs"]
mod issue1567_tests;
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
#[path = "uds_pending_swarm_tests.rs"]
mod pending_swarm_tests;
#[cfg(test)]
#[path = "uds_swarm_feedback_tests.rs"]
mod swarm_feedback_tests;
#[cfg(test)]
#[path = "uds_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "uds_workflow_automation_tests.rs"]
mod workflow_automation_tests;
