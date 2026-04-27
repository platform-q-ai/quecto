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

// ─── Prompt execution ─────────────────────────────────────────────────────────

/// Return code from [`run_agent_prompt`].
pub enum PromptOutcome {
    /// Agent completed successfully.
    Success,
    /// Agent returned an error (fatal — exit the loop).
    Error,
    /// The run was cancelled via `abort` or `steer`.
    Cancelled,
}

/// Arguments for [`run_agent_prompt`] — avoids the clippy too-many-arguments lint.
pub struct PromptArgs<'a> {
    pub agent: &'a mut AgentLoopImpl,
    pub messages: &'a mut Vec<Message>,
    pub session: &'a mut AgentSession,
    pub stdout: &'a mut (dyn tokio::io::AsyncWrite + Send + Unpin),
    pub message: Message,
    /// Oneshot cancellation receiver.  Resolves when the concurrent reader task
    /// (or `dispatch_command`) fires the cancel handle for this run.
    pub cancel_rx: tokio::sync::oneshot::Receiver<()>,
}

/// Run a single agent prompt, emitting UDS events including streamed tokens.
///
/// Installs a progress callback that forwards `AgentProgressEvent::Token`
/// events through an unbounded channel.  `agent.process()` is raced against
/// both the cancellation oneshot and the token-forwarding drain loop so that
/// tokens are emitted in real time (not buffered until completion).
pub async fn run_agent_prompt(args: PromptArgs<'_>) -> PromptOutcome {
    let PromptArgs {
        agent,
        messages,
        session: agent_session,
        stdout,
        message,
        cancel_rx,
    } = args;

    agent_session.set_streaming(true);
    emit_event(stdout, &AgentEvent::AgentStart).await;
    emit_event(stdout, &AgentEvent::TurnStart).await;

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
    let result = run_with_token_drain(TokenDrainArgs {
        agent,
        messages,
        progress_rx: &mut progress_rx,
        stdout,
        cancel_rx,
    })
    .await;

    // Clear the callback so it doesn't hold the closed sender.
    agent.set_progress_callback(None);
    agent_session.set_streaming(false);

    match result {
        None => {
            messages.truncate(user_msg_idx);
            PromptOutcome::Cancelled
        }
        Some(Ok(agent_result)) => {
            // Tool events are now forwarded in real-time via
            // forward_progress_event — no post-hoc emit needed.
            let total = agent_result
                .input_tokens
                .saturating_add(agent_result.output_tokens);
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
                },
                tool_results: vec![],
            };
            emit_event(stdout, &turn_end).await;
            let run_msgs: Vec<serde_json::Value> =
                messages[before_len..].iter().map(message_to_json).collect();
            emit_event(stdout, &AgentEvent::AgentEnd { messages: run_msgs }).await;
            PromptOutcome::Success
        }
        Some(Err(e)) => {
            emit_event(
                stdout,
                &AgentEvent::err(None, "agent_error", format!("{e}")),
            )
            .await;
            PromptOutcome::Error
        }
    }
}

/// Arguments for [`run_with_token_drain`].
struct TokenDrainArgs<'a> {
    agent: &'a mut AgentLoopImpl,
    messages: &'a mut Vec<Message>,
    progress_rx: &'a mut tokio::sync::mpsc::Receiver<AgentProgressEvent>,
    stdout: &'a mut (dyn tokio::io::AsyncWrite + Send + Unpin),
    cancel_rx: tokio::sync::oneshot::Receiver<()>,
}

/// Run `agent.process()` while draining progress events (especially tokens)
/// to the UDS writer in real time.  Races against a cancellation oneshot.
async fn run_with_token_drain(
    args: TokenDrainArgs<'_>,
) -> Option<Result<crate::domain::agent::AgentResult, crate::domain::error::DomainError>> {
    let TokenDrainArgs {
        agent,
        messages,
        progress_rx,
        stdout,
        cancel_rx,
    } = args;
    // We can't run process() and drain the channel truly concurrently because
    // process() takes &mut messages (exclusive borrow).  Instead we poll
    // both futures in a select! loop.
    tokio::pin!(cancel_rx);
    let mut process_fut = agent.process(messages);

    loop {
        tokio::select! {
            biased;  // prioritise cancel and progress over process completion
            _ = &mut cancel_rx => return None,
            Some(ev) = progress_rx.recv() => {
                forward_progress_event(ev, stdout).await;
            }
            result = &mut process_fut => {
                // Drain any remaining events that arrived between last poll
                // and process completion.
                while let Ok(ev) = progress_rx.try_recv() {
                    forward_progress_event(ev, stdout).await;
                }
                return Some(result);
            }
        }
    }
}

/// Forward a single progress event to the UDS writer.
///
/// Forwards `Token`, `ToolStarted`, and `ToolFinished` events in real time.
/// `Thinking` and `Done` are not forwarded (they have no UDS event mapping).
pub(crate) async fn forward_progress_event(
    ev: AgentProgressEvent,
    stdout: &mut (dyn tokio::io::AsyncWrite + Send + Unpin),
) {
    match ev {
        AgentProgressEvent::Token(t) => {
            emit_event(stdout, &AgentEvent::Token { token: t }).await;
        }
        AgentProgressEvent::ToolStarted {
            tool_call_id,
            name,
            arguments,
        } => {
            let args: serde_json::Value = serde_json::from_str(&arguments).unwrap_or_default();
            emit_event(
                stdout,
                &AgentEvent::ToolExecutionStart {
                    tool_call_id,
                    tool_name: name,
                    args,
                },
            )
            .await;
        }
        AgentProgressEvent::ToolFinished {
            tool_call_id,
            name,
            result_content,
            is_error,
            ..
        } => {
            emit_event(
                stdout,
                &AgentEvent::ToolExecutionEnd {
                    tool_call_id,
                    tool_name: name,
                    result: ToolResultContent {
                        content: vec![serde_json::json!({"type":"text","text": result_content})],
                    },
                    is_error,
                },
            )
            .await;
        }
        _ => {}
    }
}

/// Write an event as a JSON line followed by a newline.
pub async fn emit_event(
    writer: &mut (dyn tokio::io::AsyncWrite + Send + Unpin),
    event: &AgentEvent,
) {
    use tokio::io::AsyncWriteExt;
    let line = event.to_json_line() + "\n";
    let _ = writer.write_all(line.as_bytes()).await;
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
