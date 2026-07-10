//! Cancellation infrastructure and prompt execution for the UDS agent loop.
//!
//! The [`CancelSlot`] / [`CancelHandle`] state machine makes steer/abort
//! race-free: whether the cancel signal arrives before or during a prompt run,
//! the correct outcome (skipped or interrupted) is guaranteed.

use std::sync::Arc;

use crate::application::agent_loop::AgentLoopImpl;
use crate::domain::agent::{AgentLoop, AgentProgressEvent};
use crate::domain::message::Message;
use crate::interface::cli::protocol::{AgentEvent, ToolResultContent, TurnMessage, TurnUsage};
use crate::interface::cli::uds_session::{AgentSession, message_to_json};

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
    #[cfg(test)]
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
    /// Uses the capped serializer so a turn near a full context window can
    /// never produce a line the TUI client would drop unread (#1047).
    pub(crate) async fn emit(&mut self, event: &AgentEvent) {
        let mut line = event.to_capped_json_line();
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
    pub session: &'a mut AgentSession,
    /// Sink the streamed events are delivered to (writer XOR broadcast).
    pub sink: &'a mut EventSink<'s>,
    pub message: Message,
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
fn prompt_position(messages: &[Message], prompt_id: uuid::Uuid) -> Option<usize> {
    messages
        .iter()
        .position(|message| message.id() == prompt_id)
}

fn messages_after(messages: &[Message], prompt_id: uuid::Uuid) -> &[Message] {
    prompt_position(messages, prompt_id)
        .map(|index| &messages[index + 1..])
        .unwrap_or_default()
}

fn rollback_prompt(messages: &mut Vec<Message>, prompt_id: uuid::Uuid) {
    if let Some(index) = prompt_position(messages, prompt_id) {
        messages.truncate(index);
    }
}

pub(crate) async fn run_agent_message(args: PromptRun<'_, '_>) -> PromptOutcome {
    let PromptRun {
        agent,
        messages,
        session: agent_session,
        sink,
        message,
        cancel_rx,
        notification_rx,
        subagent_registry,
    } = args;

    agent_session.set_streaming(true);
    sink.emit(&AgentEvent::AgentStart).await;
    sink.emit(&AgentEvent::TurnStart).await;

    let prompt_id = message.id();
    messages.push(message);

    // Install a progress callback that forwards events to a bounded channel.
    // Capacity 256 limits back-pressure from a slow UDS consumer while being
    // large enough to never block under normal streaming throughput.
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel::<AgentProgressEvent>(256);
    agent.set_progress_callback(Some(Arc::new(move |ev| {
        // try_send: drop event if the channel is full rather than blocking
        // the synchronous callback.  Dropped tokens are acceptable — the
        // full text is still delivered in the turn_end event.
        let _ = progress_tx.try_send(ev);
    })));

    // Run process() + token drain concurrently, with cancel support.
    let (result, notifications) = run_with_token_drain(TokenDrainArgs {
        agent,
        messages,
        progress_rx: &mut progress_rx,
        sink,
        cancel_rx,
        notification_rx,
        subagent_registry,
    })
    .await;

    // Clear the callback so it doesn't hold the closed sender.
    agent.set_progress_callback(None);
    agent_session.set_streaming(false);

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
            rollback_prompt(messages, prompt_id);
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
            let turn_end = AgentEvent::TurnEnd {
                message: TurnMessage {
                    role: "assistant".to_string(),
                    content: agent_result.response.clone(),
                    usage,
                    stop_reason: None,
                    context_tokens: Some(agent_result.context_tokens as u64),
                    max_context_tokens: Some(agent.max_context_tokens() as u64),
                },
                tool_results: vec![],
            };
            sink.emit(&turn_end).await;
            let run_msgs: Vec<serde_json::Value> = messages_after(messages, prompt_id)
                .iter()
                .map(message_to_json)
                .collect();
            sink.emit(&AgentEvent::AgentEnd { messages: run_msgs })
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
) {
    let TokenDrainArgs {
        agent,
        messages,
        progress_rx,
        sink,
        cancel_rx,
        notification_rx,
        subagent_registry,
    } = args;
    // We can't run process() and drain the channel truly concurrently because
    // process() takes &mut messages (exclusive borrow).  Instead we poll
    // the futures in a select! loop.
    tokio::pin!(cancel_rx);
    let mut process_fut = agent.process(messages);
    let mut notifications = Vec::new();

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
                forward_progress_event_sink(ev, sink).await;
            }
            Some(notif) = notif_recv => {
                collect_notification(notif, sink, subagent_registry, &mut notifications);
            }
            result = &mut process_fut => {
                // Drain any remaining events that arrived between last poll
                // and process completion.
                while let Ok(ev) = progress_rx.try_recv() {
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

    (result, notifications)
}

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
            &notif.dedupe_key().0,
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
    let suppress = crate::infrastructure::tools::subagent_registry::consume_await_dedupe(
        subagent_registry,
        &agent_id,
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
/// Forwards `Token`, `ToolStarted`, `ToolFinished`, and `TurnCompleted` events
/// in real time. `Thinking` and `Done` are not forwarded (no UDS mapping).
pub(crate) async fn forward_progress_event_sink(ev: AgentProgressEvent, sink: &mut EventSink<'_>) {
    match ev {
        AgentProgressEvent::Token(t) => {
            sink.emit(&AgentEvent::Token { token: t }).await;
        }
        AgentProgressEvent::ToolStarted {
            tool_call_id,
            name,
            arguments,
        } => {
            let args: serde_json::Value = serde_json::from_str(&arguments).unwrap_or_default();
            sink.emit(&AgentEvent::ToolExecutionStart {
                tool_call_id,
                tool_name: name,
                args,
            })
            .await;
        }
        AgentProgressEvent::ToolFinished {
            tool_call_id,
            name,
            result_content,
            is_error,
            ..
        } => {
            sink.emit(&AgentEvent::ToolExecutionEnd {
                tool_call_id,
                tool_name: name,
                result: ToolResultContent {
                    content: vec![serde_json::json!({"type":"text","text": result_content})],
                },
                is_error,
            })
            .await;
        }
        AgentProgressEvent::TurnCompleted { messages } => {
            // Stream this turn's output (assistant + tool results) on the agent's
            // own stream; a parent monitor re-stamps it with the child id (#797).
            let json: Vec<serde_json::Value> = messages.iter().map(message_to_json).collect();
            sink.emit(&AgentEvent::SubagentMessagesAppended {
                agent_id: String::new(),
                messages: json,
            })
            .await;
        }
        _ => {}
    }
}

/// Thin `Writer`-sink adapter for progress events, retained for unit tests that
/// assert on raw writer bytes.
#[cfg(test)]
pub(crate) async fn forward_progress_event(
    ev: AgentProgressEvent,
    stdout: &mut (dyn tokio::io::AsyncWrite + Send + Unpin),
) {
    forward_progress_event_sink(ev, &mut EventSink::writer(stdout)).await;
}

#[cfg(test)]
#[path = "uds_cancel_tests.rs"]
mod tests;
