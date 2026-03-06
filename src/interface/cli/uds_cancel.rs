//! Cancellation infrastructure and prompt execution for the UDS agent loop.
//!
//! The [`CancelSlot`] / [`CancelHandle`] state machine makes steer/abort
//! race-free: whether the cancel signal arrives before or during a prompt run,
//! the correct outcome (skipped or interrupted) is guaranteed.

use std::sync::Arc;

use crate::application::agent_loop::AgentLoopImpl;
use crate::domain::agent::{AgentLoop, AgentProgressEvent};
use crate::domain::message::Message;
use crate::interface::cli::protocol::{AgentEvent, ToolResultContent, TurnMessage};
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
    pub message: String,
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
    messages.push(Message::user(message));
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
            let turn_end = AgentEvent::TurnEnd {
                message: TurnMessage {
                    role: "assistant".to_string(),
                    content: agent_result.response.clone(),
                    usage: None,
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
            is_error,
            ..
        } => {
            emit_event(
                stdout,
                &AgentEvent::ToolExecutionEnd {
                    tool_call_id,
                    tool_name: name,
                    result: ToolResultContent {
                        content: vec![serde_json::json!({"type":"text","text":""})],
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
