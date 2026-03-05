//! Cancellation infrastructure and prompt execution for the UDS agent loop.
//!
//! The [`CancelSlot`] / [`CancelHandle`] state machine makes steer/abort
//! race-free: whether the cancel signal arrives before or during a prompt run,
//! the correct outcome (skipped or interrupted) is guaranteed.

use crate::application::agent_loop::AgentLoopImpl;
use crate::domain::agent::AgentLoop;
use crate::domain::message::{Message, Role};
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

/// Run a single agent prompt, emitting UDS events.
///
/// Races `agent.process()` against a oneshot cancellation signal.  If the
/// cancel fires first, the in-flight HTTP request is dropped immediately
/// (the reqwest future is cancelled at the OS level) and the partially-appended
/// user message is removed from `messages`.
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

    // Race process() against the oneshot cancel signal.  If cancel fires, the
    // process() future is dropped — reqwest aborts the in-flight HTTP request.
    let result = tokio::select! {
        res = agent.process(messages) => Some(res),
        _ = cancel_rx => None,
    };

    agent_session.set_streaming(false);

    match result {
        None => {
            // Cancelled — remove the user message (and any partial assistant /
            // tool messages that process() may have appended before cancellation).
            messages.truncate(user_msg_idx);
            PromptOutcome::Cancelled
        }
        Some(Ok(agent_result)) => {
            emit_tool_events(stdout, &messages[before_len..]).await;

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

/// Emit `tool_execution_start` / `tool_execution_end` events from a message slice.
async fn emit_tool_events(
    stdout: &mut (dyn tokio::io::AsyncWrite + Send + Unpin),
    messages: &[Message],
) {
    for msg in messages {
        if msg.role == Role::Assistant {
            for tc in &msg.tool_calls {
                let args: serde_json::Value =
                    serde_json::from_str(&tc.arguments).unwrap_or_default();
                emit_event(
                    stdout,
                    &AgentEvent::ToolExecutionStart {
                        tool_call_id: tc.id.clone(),
                        tool_name: tc.name.clone(),
                        args,
                    },
                )
                .await;
            }
        } else if msg.role == Role::Tool {
            emit_event(
                stdout,
                &AgentEvent::ToolExecutionEnd {
                    tool_call_id: msg.tool_call_id.clone().unwrap_or_default(),
                    tool_name: msg.tool_name.clone().unwrap_or_default(),
                    result: ToolResultContent {
                        content: vec![serde_json::json!({"type":"text","text": msg.content})],
                    },
                    is_error: msg.is_error,
                },
            )
            .await;
        }
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
