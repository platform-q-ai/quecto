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
    /// Write JSON lines directly to an async writer (single client).
    Writer(&'a mut (dyn tokio::io::AsyncWrite + Send + Unpin)),
    /// Fan JSON lines out to every connected client (multi-client server).
    Broadcast(tokio::sync::broadcast::Sender<String>),
}

impl<'a> EventSink<'a> {
    /// Build a writer sink from a borrowed async writer.
    pub(crate) fn writer(w: &'a mut (dyn tokio::io::AsyncWrite + Send + Unpin)) -> Self {
        EventSink::Writer(w)
    }

    /// Serialize an event to a JSON line and deliver it via this sink.
    pub(crate) async fn emit(&mut self, event: &AgentEvent) {
        let mut line = event.to_json_line();
        line.push('\n');
        match self {
            EventSink::Writer(w) => {
                use tokio::io::AsyncWriteExt;
                let _ = w.write_all(line.as_bytes()).await;
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
            EventSink::Writer(_) => None,
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

    let user_msg_idx = messages.len();
    messages.push(message);
    let before_len = messages.len();

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
            messages.truncate(user_msg_idx);
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
            let run_msgs: Vec<serde_json::Value> =
                messages[before_len..].iter().map(message_to_json).collect();
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
    forward_progress_event_sink(ev, &mut EventSink::Writer(stdout)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_handle() -> CancelHandle {
        std::sync::Arc::new(std::sync::Mutex::new(CancelSlot::Idle))
    }

    #[test]
    fn abort_then_prompt_works() {
        // Simulates the full abort flow: reader fires cancel on the
        // running prompt, then handle_abort is dispatched.
        // After both, the next prompt should NOT be pre-cancelled.
        let handle = make_handle();

        // Arm for the current run.
        let _rx = arm_cancel(&handle).expect("should arm");

        // Reader task fires cancel (kills running prompt).
        fire_cancel(&handle);

        // handle_abort dispatches (should NOT fire again after fix).
        // Before fix: fire_cancel(&handle) was called here too.
        // After fix: handle_abort only emits the ack event.

        // Next prompt arms successfully.
        let result = arm_cancel(&handle);
        assert!(
            result.is_some(),
            "next prompt should arm successfully after abort"
        );
    }

    #[test]
    fn single_fire_allows_next_prompt() {
        // After the fix: only one fire_cancel (reader task).
        // The next arm_cancel should succeed.
        let handle = make_handle();

        // Arm for the current run.
        let _rx = arm_cancel(&handle).expect("should arm");

        // Single fire (reader task only).
        fire_cancel(&handle);
        // Slot is now Idle.

        // Next prompt arms successfully.
        let result = arm_cancel(&handle);
        assert!(result.is_some(), "single fire should allow next arm_cancel");
    }

    #[test]
    fn fire_on_idle_pre_cancels() {
        let handle = make_handle();
        fire_cancel(&handle);
        assert!(
            arm_cancel(&handle).is_none(),
            "Fired slot should pre-cancel"
        );
    }

    #[test]
    fn arm_disarm_cycle() {
        let handle = make_handle();
        let _rx = arm_cancel(&handle).expect("should arm");
        disarm_cancel(&handle);
        // Should be back to Idle.
        let rx2 = arm_cancel(&handle);
        assert!(rx2.is_some(), "should re-arm after disarm");
    }

    #[test]
    fn fire_on_already_fired_is_noop() {
        let handle = make_handle();
        fire_cancel(&handle); // Idle → Fired
        fire_cancel(&handle); // Fired → Fired (noop)
        // Should still pre-cancel next arm.
        assert!(arm_cancel(&handle).is_none());
    }

    #[test]
    fn disarm_on_idle_is_noop() {
        let handle = make_handle();
        disarm_cancel(&handle); // nothing to disarm
        // Should still arm normally.
        let rx = arm_cancel(&handle);
        assert!(rx.is_some());
    }

    #[test]
    fn disarm_on_fired_does_not_clear() {
        let handle = make_handle();
        fire_cancel(&handle); // → Fired
        disarm_cancel(&handle); // Fired is not Armed, so this is a no-op
        // Slot should still be Fired.
        assert!(
            arm_cancel(&handle).is_none(),
            "Fired state should survive disarm"
        );
    }

    #[test]
    fn arm_cancel_returns_receiver() {
        let handle = make_handle();
        let rx = arm_cancel(&handle).expect("should arm");
        // Fire should signal the receiver.
        fire_cancel(&handle);
        // rx should be signalled (sender dropped or sent).
        // In a real scenario we'd poll the receiver.
        drop(rx); // just verify it exists
    }

    #[test]
    fn multiple_arm_disarm_cycles() {
        let handle = make_handle();
        for _ in 0..10 {
            let _rx = arm_cancel(&handle).expect("should arm");
            disarm_cancel(&handle);
        }
        // Final arm should still work.
        assert!(arm_cancel(&handle).is_some());
    }

    #[test]
    fn fire_then_arm_resets_to_idle() {
        let handle = make_handle();
        fire_cancel(&handle); // → Fired
        let result = arm_cancel(&handle); // Fired → Idle, returns None
        assert!(result.is_none());
        // Now slot should be Idle, so next arm succeeds.
        let result2 = arm_cancel(&handle);
        assert!(result2.is_some(), "should arm after Fired was consumed");
    }
}
