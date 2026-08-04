//! Cancellation infrastructure and prompt execution for the UDS agent loop.
//!
//! The [`CancelSlot`] / [`CancelHandle`] state machine makes steer/abort
//! race-free: whether the cancel signal arrives before or during a prompt run,
//! the correct outcome (skipped or interrupted) is guaranteed.

use super::uds_snapshots::user_visible_messages;
use std::sync::Arc;

use crate::application::agent_loop::AgentLoopImpl;
use crate::domain::agent::{AgentLoop, AgentProgressEvent};
use crate::domain::message::Message;
use crate::interface::cli::protocol::{AgentEvent, TurnMessage, TurnUsage};
use crate::interface::cli::uds_session::AgentSession;

/// State of the cancellation slot for the current (or next) agent run.
pub enum CancelSlot {
    /// Idle — no run is in progress and no cancel has been requested.
    Idle,
    /// A run is in progress; drop this sender to cancel it.
    Armed(tokio::sync::oneshot::Sender<()>),
    /// Cancel was requested before the run started (or while arming).
    /// The next call to [`arm_cancel`] will immediately return `None`.
    Fired,
}

/// Shared cancellation state protected by a mutex.
pub type CancelHandle = std::sync::Arc<std::sync::Mutex<CancelSlot>>;

/// Cross-task signalling for explicit control verbs that race the post-turn
/// workflow auto-continue nudge (#895/#896).
///
/// The reader task classifies `abort`/`steer` lines and sets these flags BEFORE
/// the corresponding command is dispatched, so the in-flight prompt's
/// `drain_pending_and_nudge` (which runs at the idle boundary, before the
/// queued command is handled) can honour the operator's intent:
///
/// - `abort_requested` = full stop (#895): suppress the workflow auto-continue
///   nudge and discard any queued work so the bound workflow does NOT resume.
/// - `steer_pending` = explicit redirect (#896): the auto-continue nudge yields
///   so the steered instruction is obeyed next instead of being overridden.
#[derive(Default)]
pub struct TurnControl {
    abort_requested: std::sync::atomic::AtomicBool,
    steer_pending: std::sync::atomic::AtomicBool,
}

impl TurnControl {
    /// Reader: record a full-stop abort ahead of dispatch (#895).
    pub fn mark_abort(&self) {
        self.abort_requested
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Reader: record an explicit steer ahead of dispatch (#896).
    pub fn mark_steer(&self) {
        self.steer_pending
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Idle drain: consume the abort flag (true once, then cleared).
    pub fn take_abort(&self) -> bool {
        self.abort_requested
            .swap(false, std::sync::atomic::Ordering::SeqCst)
    }

    /// Whether a full-stop abort is queued but not yet consumed.
    pub fn is_abort_pending(&self) -> bool {
        self.abort_requested
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Whether a steer is queued but not yet handled.
    pub fn is_steer_pending(&self) -> bool {
        self.steer_pending.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Release the steer gate (its handler is now running).
    pub fn clear_steer(&self) {
        self.steer_pending
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }

    /// Clear both flags (abort handler / full reset).
    pub fn clear(&self) {
        self.abort_requested
            .store(false, std::sync::atomic::Ordering::SeqCst);
        self.steer_pending
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Shared handle to [`TurnControl`], cloned into the reader task.
pub type TurnControlHandle = std::sync::Arc<TurnControl>;

/// Attempt to fire cancellation.
///
/// - `Armed`: fires the sender (cancels the running prompt immediately).
/// - `Idle`:  transitions to `Fired` so the next `arm_cancel` is pre-cancelled.
/// - `Fired`: no-op (already pending).
///
/// # Panics
/// Panics if the `CancelSlot` mutex is poisoned (indicates a bug elsewhere).
pub fn fire_cancel(handle: &CancelHandle) {
    let mut guard = handle.lock().expect("CancelSlot mutex poisoned");
    match std::mem::replace(&mut *guard, CancelSlot::Idle) {
        CancelSlot::Armed(tx) => {
            let _ = tx.send(());
            // Slot is now Idle — the prompt cleans up after the select!.
        }
        CancelSlot::Idle => {
            *guard = CancelSlot::Fired;
        }
        CancelSlot::Fired => {
            *guard = CancelSlot::Fired; // restore
        }
    }
}

/// Arm the cancellation slot for an upcoming run.
///
/// Returns `Some(rx)` when armed — the caller should race `rx` against
/// `process()` in a `select!`.
///
/// Returns `None` when the slot was already `Fired` (a cancel arrived before
/// the run started) — the caller should skip the run entirely.
///
/// # Panics
/// Panics if the `CancelSlot` mutex is poisoned (indicates a bug elsewhere).
pub fn arm_cancel(handle: &CancelHandle) -> Option<tokio::sync::oneshot::Receiver<()>> {
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let mut guard = handle.lock().expect("CancelSlot mutex poisoned");
    match *guard {
        CancelSlot::Fired => {
            *guard = CancelSlot::Idle;
            None
        }
        _ => {
            *guard = CancelSlot::Armed(tx);
            Some(rx)
        }
    }
}

/// Disarm and reset the slot after a run completes normally.
///
/// # Panics
/// Panics if the `CancelSlot` mutex is poisoned (indicates a bug elsewhere).
pub fn disarm_cancel(handle: &CancelHandle) {
    let mut guard = handle.lock().expect("CancelSlot mutex poisoned");
    if matches!(*guard, CancelSlot::Armed(_)) {
        *guard = CancelSlot::Idle;
    }
}

// ─── Event sink ───────────────────────────────────────────────────────────────

use crate::infrastructure::tools::subagent_registry::{
    NotificationRx, SequencedSubagentNotification, SubagentRegistry,
};

/// Where a prompt's streaming events are delivered (#994).
///
/// Unifies the two historical pipelines — a direct async writer (single-client
/// / test path) and the multi-client broadcast channel — so there is ONE prompt
/// implementation parameterized by this sink. Production takes `Broadcast`; the
/// `Writer` variant is the trivial single-client/test case.
pub(crate) enum EventSink<'a> {
    /// Write JSON messages directly to an async writer (single client), in
    /// the connection's negotiated framing (#1059).
    Writer(
        &'a mut (dyn tokio::io::AsyncWrite + Send + Unpin),
        super::uds_wire::ConnectionWireMode,
    ),
    /// Fan JSON messages out to every connected client (multi-client server).
    /// Lines stay newline-terminated on this channel; each client's writer
    /// task re-frames them for its own connection (#1059).
    Broadcast(tokio::sync::broadcast::Sender<String>),
}

impl<'a> EventSink<'a> {
    /// Build a writer sink from a borrowed async writer. Writes legacy NDJSON
    /// (non-negotiating test paths that assert on raw writer bytes).
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn writer(w: &'a mut (dyn tokio::io::AsyncWrite + Send + Unpin)) -> Self {
        Self::writer_with_mode(w, super::uds_wire::ConnectionWireMode::legacy())
    }

    /// Build a writer sink that replies in a UDS connection's negotiated
    /// framing (#1059).
    pub(crate) fn writer_with_mode(
        w: &'a mut (dyn tokio::io::AsyncWrite + Send + Unpin),
        mode: super::uds_wire::ConnectionWireMode,
    ) -> Self {
        EventSink::Writer(w, mode)
    }

    /// Serialize an event to a JSON line and deliver it via this sink.
    ///
    /// Event payloads are bounded by construction. An over-cap event is an
    /// invariant violation, so reject it whole rather than reshaping content.
    pub(crate) async fn emit(&mut self, event: &AgentEvent) {
        self.emit_serialized(event.to_json_line()).await;
    }

    /// Deliver an ALREADY-SERIALIZED event line (without trailing newline),
    /// applying the same over-cap rejection as [`EventSink::emit`]. Callers
    /// that must inspect the serialized length themselves (e.g. the response
    /// frame-limit guard) use this to avoid serializing twice.
    pub(crate) async fn emit_serialized(&mut self, mut line: String) {
        if line.len() > super::protocol::EVENT_LINE_JSON_BUDGET {
            tracing::warn!(
                len = line.len(),
                cap = super::protocol::EVENT_LINE_CAP_BYTES,
                "dropping oversized outbound event"
            );
            return;
        }
        line.push('\n');
        match self {
            EventSink::Writer(w, mode) => {
                let _ = super::uds_wire::write_event_line(&mut **w, &line, mode).await;
            }
            EventSink::Broadcast(tx) => {
                let _ = tx.send(line);
            }
        }
    }

    /// The broadcast sender, when this is a broadcast sink. Subagent
    /// notifications are a multi-client concern and only ever fire on this
    /// variant.
    pub(crate) async fn emit_ledger_advanced(
        &mut self,
        advance: super::uds_snapshots::LedgerAdvance,
    ) {
        if advance.changed {
            self.emit_serialized(
                serde_json::json!({
                    "type": "ledger_advanced",
                    "epoch": advance.epoch,
                    "rev": advance.rev,
                })
                .to_string(),
            )
            .await;
        }
    }

    fn broadcast_sender(&self) -> Option<&tokio::sync::broadcast::Sender<String>> {
        match self {
            EventSink::Broadcast(tx) => Some(tx),
            EventSink::Writer(..) => None,
        }
    }
}

// ─── Prompt execution ─────────────────────────────────────────────────────────

/// Return code from [`run_agent_message`].
pub enum PromptOutcome {
    /// Agent completed successfully.
    ///
    /// Deliberately carries NO durable-prefix-dirty payload (#1073 review):
    /// the agent-level latch (`AgentLoopImpl::take_durable_prefix_dirty`,
    /// drained centrally by `persist_current_session`) is the single
    /// authoritative channel. A result-carried flag existed briefly but was
    /// success-only — dirtiness from an Error/Cancelled turn would be lost by
    /// any consumer that trusted it — and no production code ever read it.
    Success,
    /// Agent returned an error (fatal — exit the loop).
    Error,
    /// The run was cancelled via `abort` or `steer`.
    Cancelled,
}

/// Arguments for [`run_agent_message`] — the single, sink-parameterized prompt
/// pipeline. Avoids the clippy too-many-arguments lint.
pub(crate) struct PromptRun<'a, 's> {
    pub agent: &'a mut AgentLoopImpl,
    pub messages: &'a mut Vec<Message>,
    /// Shared live ledger used by busy-path get_message readers.
    pub conversation_snapshot: Option<super::uds_multi::ConversationSnapshot>,
    pub execution_state: Option<super::uds_execution_state::ExecutionStateHandle>,
    pub session: &'a mut AgentSession,
    /// Sink the streamed events are delivered to (writer XOR broadcast).
    pub sink: &'a mut EventSink<'s>,
    pub message: Message,
    /// System prompt to hide from user-visible busy snapshots.
    pub system_prompt: &'a str,
    /// Oneshot cancellation receiver.  Resolves when the concurrent reader task
    /// (or `dispatch_command`) fires the cancel handle for this run.
    pub cancel_rx: tokio::sync::oneshot::Receiver<()>,
    /// Subagent notification receiver — drained during prompt execution (#534).
    /// `&mut None` on the single-client / writer path (no subagents).
    pub notification_rx: &'a mut Option<NotificationRx>,
    /// Subagent registry for building state-changed events (#534). `&None` on
    /// the single-client path.
    pub subagent_registry: &'a Option<SubagentRegistry>,
}

/// Run a single agent prompt, emitting UDS events (including streamed tokens)
/// to `sink`.
///
/// Installs a progress callback that forwards `AgentProgressEvent`s through a
/// bounded channel. `agent.process()` is raced against the cancellation
/// oneshot, the token-forwarding drain loop, and (on a broadcast sink) subagent
/// notifications, so tokens/notes are emitted in real time — not buffered until
/// completion.
pub(crate) async fn run_agent_message(args: PromptRun<'_, '_>) -> PromptOutcome {
    let PromptRun {
        agent,
        messages,
        conversation_snapshot,
        execution_state,
        session: agent_session,
        sink,
        message,
        system_prompt,
        cancel_rx,
        notification_rx,
        subagent_registry,
    } = args;

    agent_session.set_streaming(true);
    if let Some(state) = &execution_state {
        if let Ok(mut state) = state.lock() {
            state.start_run();
        }
    }
    sink.emit(&AgentEvent::AgentStart).await;
    sink.emit(&AgentEvent::TurnStart).await;

    let prompt_id = message.id();
    messages.push(message);
    if let Some(state) = &execution_state {
        if let Ok(mut state) = state.lock() {
            let visible_count = user_visible_messages(messages, system_prompt).len();
            state.set_hidden_message_count(messages.len().saturating_sub(visible_count));
            state.set_message_count(visible_count);
        }
    }
    if let Some(snapshot) = &conversation_snapshot {
        let visible = super::uds_snapshots::user_visible_messages(messages, system_prompt);
        let advance = snapshot.write().await.publish(&visible);
        sink.emit_ledger_advanced(advance).await;
    }

    // Install a progress callback that forwards events to a bounded channel.
    // Capacity 256 limits back-pressure from a slow UDS consumer while being
    // large enough to never block under normal streaming throughput.
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel::<AgentProgressEvent>(256);
    let live_execution = execution_state.clone();
    agent.set_progress_callback(Some(Arc::new(move |ev| {
        // Operational telemetry is recorded synchronously before the lossy UI
        // channel, so a saturated event stream cannot hide tool/message progress.
        if let Some(state) = &live_execution {
            if let Ok(mut state) = state.lock() {
                state.observe(&ev);
            }
        }
        // try_send: drop event if the channel is full rather than blocking
        // the synchronous callback. Dropped presentation events are acceptable.
        let _ = progress_tx.try_send(ev);
    })));

    // Run process() + token drain concurrently, with cancel support.
    let (result, notifications, tokens_emitted) = run_with_token_drain(TokenDrainArgs {
        agent,
        messages,
        progress_rx: &mut progress_rx,
        sink,
        cancel_rx,
        notification_rx,
        subagent_registry,
        conversation_snapshot: conversation_snapshot.as_ref(),
    })
    .await;

    // The CLI UDS boundary owns a mutable agent and is therefore the real
    // production boundary where queued AtNextTurnBoundary policy mutations can
    // be drained before the next prompt snapshots tool definitions.
    let _ = agent.drain_tool_policy_mutations_at_boundary();

    // Clear the callback so it doesn't hold the closed sender.
    agent.set_progress_callback(None);
    agent_session.set_streaming(false);
    if let Some(state) = &execution_state {
        if let Ok(mut state) = state.lock() {
            state.finish_run();
        }
    }

    // Auto-await (#816): completions that arrived mid-turn are buffered here as
    // pending notes — NOT injected into the turn that just ran. They surface at
    // the parent's NEXT idle boundary. Empty on the writer path.
    for notif in notifications {
        let (agent_id, sequence) = notif.dedupe_key();
        agent_session.enqueue_subagent_notification(
            agent_id,
            sequence,
            notif.to_message(),
            notif.is_completion(),
        );
    }

    match result {
        None => {
            let finalized =
                super::uds_cancel_history::finalize_interrupted_turn(messages, prompt_id);
            if let Some(snapshot) = &conversation_snapshot {
                let visible = super::uds_snapshots::user_visible_messages(messages, system_prompt);
                let mut snap = snapshot.write().await;
                let publish = snap.publish(&visible);
                let full = snap.record_full(&finalized.recordable_messages());
                drop(snap);
                sink.emit_ledger_advanced(publish).await;
                sink.emit_ledger_advanced(full).await;
            }
            if let Some(state) = &execution_state {
                if let Ok(mut state) = state.lock() {
                    state.set_message_count(user_visible_messages(messages, system_prompt).len());
                }
            }
            PromptOutcome::Cancelled
        }
        Some(Ok(agent_result)) => {
            agent_session.record_agent_result(&agent_result);
            // Tool events are forwarded in real-time via forward_progress_event
            // — emitting them again here would duplicate events with conflicting
            // IDs.
            let total = agent_result.turn_tokens();
            let usage = if total > 0 {
                Some(TurnUsage {
                    input: agent_result.input_tokens,
                    output: agent_result.output_tokens,
                    total,
                })
            } else {
                None
            };
            // #1060: non-streaming turns never emit token progress events.
            // Surface the final assistant text once as a Token so continuously
            // connected clients (and e2e assertions) still observe the response
            // without re-carrying it on turn_end / agent_end.
            if !tokens_emitted && !agent_result.response.is_empty() {
                sink.emit(&AgentEvent::Token {
                    token: agent_result.response.clone(),
                })
                .await;
            }
            // #1060 / ADR-0008 part 2: end-of-turn events carry stable message
            // refs + small footer metadata only — never re-ship full content.
            // process() has appended the completed turn to `messages`; publish
            // that ledger before refs are emitted so a concurrent reader-side
            // get_message can resolve every referenced role immediately.
            // Publish the live (post-prune) conversation for get_messages, and
            // record FULL copies of this run's appended messages into the
            // id-addressable ledger so the refs emitted below resolve via
            // get_message even after the ladder later prunes/collapses them
            // (#1060 review 1a; 1b: full publish, not a length-based extend).
            if let Some(snapshot) = &conversation_snapshot {
                let visible = super::uds_snapshots::user_visible_messages(messages, system_prompt);
                let mut snap = snapshot.write().await;
                let publish = snap.publish(&visible);
                let full = snap.record_full(&agent_result.appended_messages);
                drop(snap);
                sink.emit_ledger_advanced(publish).await;
                sink.emit_ledger_advanced(full).await;
            }
            let message_refs: Vec<String> = agent_result
                .appended_messages
                .iter()
                .map(|m| m.id().to_string())
                .collect();
            let turn_end = AgentEvent::TurnEnd {
                message: TurnMessage {
                    role: "assistant".to_string(),
                    content: String::new(),
                    message_refs: message_refs.clone(),
                    usage,
                    stop_reason: None,
                    context_tokens: Some(agent_result.context_tokens as u64),
                    max_context_tokens: Some(agent.max_context_tokens() as u64),
                    content_length: Some(agent_result.response.len() as u64),
                },
                tool_results: vec![],
            };
            sink.emit(&turn_end).await;
            sink.emit(&AgentEvent::AgentEnd {
                messages: vec![],
                message_refs,
            })
            .await;
            PromptOutcome::Success
        }
        Some(Err(e)) => {
            sink.emit(&AgentEvent::err(None, "agent_error", format!("{e}")))
                .await;
            PromptOutcome::Error
        }
    }
}

/// Arguments for [`run_with_token_drain`].
struct TokenDrainArgs<'a, 's> {
    agent: &'a mut AgentLoopImpl,
    messages: &'a mut Vec<Message>,
    progress_rx: &'a mut tokio::sync::mpsc::Receiver<AgentProgressEvent>,
    sink: &'a mut EventSink<'s>,
    cancel_rx: tokio::sync::oneshot::Receiver<()>,
    notification_rx: &'a mut Option<NotificationRx>,
    subagent_registry: &'a Option<SubagentRegistry>,
    conversation_snapshot: Option<&'a super::uds_multi::ConversationSnapshot>,
}

/// Run `agent.process()` while draining progress events (especially tokens)
/// to `sink` in real time, and (on a broadcast sink) forwarding subagent
/// notifications. Races against a cancellation oneshot.
///
/// Returns the process result (`None` when cancelled) plus any subagent
/// notifications collected mid-turn that should be enqueued as pending notes.
async fn run_with_token_drain(
    args: TokenDrainArgs<'_, '_>,
) -> (
    Option<Result<crate::domain::agent::AgentResult, crate::domain::error::DomainError>>,
    Vec<SequencedSubagentNotification>,
    bool,
) {
    let TokenDrainArgs {
        agent,
        messages,
        progress_rx,
        sink,
        cancel_rx,
        notification_rx,
        subagent_registry,
        conversation_snapshot,
    } = args;
    // We can't run process() and drain the channel truly concurrently because
    // process() takes &mut messages (exclusive borrow).  Instead we poll
    // the futures in a select! loop.
    tokio::pin!(cancel_rx);
    let mut process_fut = agent.process(messages);
    let mut notifications = Vec::new();
    let mut tokens_emitted = false;

    let result = loop {
        // Drain notification_rx if present (#534) so SubagentStateChanged events
        // reach clients in real-time during prompt execution. `pending()` when
        // absent (single-client / writer path) so this arm never fires.
        let notif_recv = async {
            if let Some(rx) = notification_rx.as_mut() {
                rx.recv().await
            } else {
                std::future::pending().await
            }
        };

        tokio::select! {
            biased;  // prioritise cancel and progress over process completion
            _ = &mut cancel_rx => break None,
            Some(ev) = progress_rx.recv() => {
                if matches!(ev, AgentProgressEvent::Token(_)) {
                    tokens_emitted = true;
                }
                publish_turn_progress(&ev, conversation_snapshot, sink).await;
                forward_progress_event_sink(ev, sink).await;
            }
            Some(notif) = notif_recv => {
                collect_notification(notif, sink, subagent_registry, &mut notifications);
            }
            result = &mut process_fut => {
                // Drain any remaining events that arrived between last poll
                // and process completion.
                while let Ok(ev) = progress_rx.try_recv() {
                    if matches!(ev, AgentProgressEvent::Token(_)) {
                        tokens_emitted = true;
                    }
                    publish_turn_progress(&ev, conversation_snapshot, sink).await;
                    forward_progress_event_sink(ev, sink).await;
                }
                if let Some(rx) = notification_rx.as_mut() {
                    while let Ok(notif) = rx.try_recv() {
                        collect_notification(notif, sink, subagent_registry, &mut notifications);
                    }
                }
                break Some(result);
            }
        }
    };

    (result, notifications, tokens_emitted)
}

#[path = "uds_turn_progress.rs"]
mod uds_turn_progress;
pub(crate) use uds_turn_progress::publish_turn_progress;

/// Handle one subagent notification received mid-turn: broadcast it to clients
/// when the sink is `Broadcast`, and collect it for LLM injection unless the
/// auto-await dedupe suppresses it. On a `Writer` sink the notification is
/// still collected (never silently dropped) — only the client fan-out is a
/// broadcast-only concern (#994).
fn collect_notification(
    notif: SequencedSubagentNotification,
    sink: &EventSink<'_>,
    subagent_registry: &Option<SubagentRegistry>,
    notifications: &mut Vec<SequencedSubagentNotification>,
) {
    let delivered = match sink.broadcast_sender() {
        Some(tx) => forward_notification_broadcast(notif.clone(), tx, subagent_registry),
        None => !crate::infrastructure::tools::subagent_registry::consume_await_dedupe(
            subagent_registry,
            &notif.await_dedupe_key().0,
        ),
    };
    if delivered {
        notifications.push(notif);
    }
}

/// Forward a subagent notification as broadcast events (#534).
///
/// Returns `true` when the passive completion note was delivered (so the caller
/// should also collect it for LLM injection), or `false` when it was SUPPRESSED
/// because a manual `await` already consumed this terminal completion
/// (auto-await dedupe). The `SubagentStateChanged` panel update is always
/// emitted regardless. Race-free because the dispatch/drain loop is
/// single-threaded: the await tool set the flag before this notification is
/// processed.
pub(in crate::interface::cli) fn forward_notification_broadcast(
    notif: SequencedSubagentNotification,
    broadcast_tx: &tokio::sync::broadcast::Sender<String>,
    subagent_registry: &Option<SubagentRegistry>,
) -> bool {
    let (agent_id, sequence) = notif.dedupe_key();
    let (dedupe_ref, _) = notif.await_dedupe_key();
    let suppress = crate::infrastructure::tools::subagent_registry::consume_await_dedupe(
        subagent_registry,
        &dedupe_ref,
    );
    if !suppress {
        tracing::info!(
            %agent_id,
            sequence,
            "broadcasting passive subagent notification during prompt"
        );
        let ev = AgentEvent::SubagentNotification {
            agent_id,
            sequence,
            message: notif.to_message(),
        };
        let mut line = ev.to_json_line();
        line.push('\n');
        let _ = broadcast_tx.send(line);
    }
    // Build full subagent info list from registry for the state-changed event.
    let list = crate::interface::cli::protocol::build_subagent_info_list(subagent_registry);
    let ev = AgentEvent::SubagentStateChanged { subagents: list };
    let mut line = ev.to_json_line();
    line.push('\n');
    let _ = broadcast_tx.send(line);
    !suppress
}

/// Forward a single progress event to `sink`.
///
/// Forwards progress events in real time. `Thinking` and `Done` are not forwarded
/// (no UDS mapping).
pub(crate) async fn forward_progress_event_sink(ev: AgentProgressEvent, sink: &mut EventSink<'_>) {
    super::uds_progress_forward::forward_event(ev, sink).await;
}

/// Thin `Writer`-sink adapter for progress events, retained for unit tests that
/// assert on raw writer bytes.
#[cfg(any(test, feature = "test-support"))]
pub async fn forward_progress_event(
    ev: AgentProgressEvent,
    stdout: &mut (dyn tokio::io::AsyncWrite + Send + Unpin),
) {
    forward_progress_event_sink(ev, &mut EventSink::writer(stdout)).await;
}

#[cfg(test)]
#[path = "uds_cancel_1060_tests.rs"]
mod issue_1060_tests;
#[cfg(test)]
#[path = "uds_1072_e2e_tests.rs"]
mod issue_1072_e2e_tests;
#[cfg(test)]
#[path = "uds_cancel_tests.rs"]
mod tests;
