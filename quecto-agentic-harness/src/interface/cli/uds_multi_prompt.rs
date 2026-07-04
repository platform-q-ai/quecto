use super::*;
use crate::interface::cli::{protocol, uds};

pub(crate) struct PromptArgsBroadcast<'a> {
    pub agent: &'a mut AgentLoopImpl,
    pub messages: &'a mut Vec<Message>,
    pub session: &'a mut AgentSession,
    pub broadcast_tx: tokio::sync::broadcast::Sender<String>,
    pub message: Message,
    pub cancel_rx: tokio::sync::oneshot::Receiver<()>,
    /// Subagent notification receiver — drained during prompt execution (#534).
    pub notification_rx:
        &'a mut Option<crate::infrastructure::tools::subagent_registry::NotificationRx>,
    /// Subagent registry for building state-changed events (#534).
    pub subagent_registry:
        &'a Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
}

/// Run an agent prompt, emitting events to the broadcast channel.
pub(crate) async fn run_agent_prompt_broadcast(args: PromptArgsBroadcast<'_>) -> PromptOutcome {
    use crate::domain::agent::AgentProgressEvent;
    use crate::interface::cli::protocol::{TurnMessage, TurnUsage};

    let PromptArgsBroadcast {
        agent,
        messages,
        session,
        broadcast_tx,
        message,
        cancel_rx,
        notification_rx,
        subagent_registry,
    } = args;

    session.set_streaming(true);
    broadcast_event(&broadcast_tx, &AgentEvent::AgentStart);
    broadcast_event(&broadcast_tx, &AgentEvent::TurnStart);

    let user_msg_idx = messages.len();
    messages.push(message);
    let before_len = messages.len();

    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel::<AgentProgressEvent>(256);
    agent.set_progress_callback(Some(std::sync::Arc::new(move |ev| {
        let _ = progress_tx.try_send(ev);
    })));

    let drain_result = run_with_token_drain_broadcast(TokenDrainBroadcastArgs {
        broadcast_tx: &broadcast_tx,
        agent,
        messages,
        progress_rx: &mut progress_rx,
        cancel_rx,
        notification_rx,
        subagent_registry,
    })
    .await;

    agent.set_progress_callback(None);
    session.set_streaming(false);

    // Auto-await (#816): completions that arrived mid-turn are buffered here as
    // pending notes — NOT injected into the turn that just ran. They surface at
    // the parent's NEXT idle boundary, drained by `drain_pending_and_nudge`
    // immediately after this prompt returns. This is what guarantees a note never
    // appears mid-turn. `enqueue_subagent_notification` dedupes/coalesces per
    // agent internally.
    for notif in drain_result.notifications {
        let (agent_id, sequence) = notif.dedupe_key();
        session.enqueue_subagent_notification(
            agent_id,
            sequence,
            notif.to_message(),
            notif.is_completion(),
        );
    }

    match drain_result.result {
        None => {
            messages.truncate(user_msg_idx);
            PromptOutcome::Cancelled
        }
        Some(Ok(agent_result)) => {
            session.record_agent_result(&agent_result);
            // Post-hoc tool events are NOT emitted here — ToolStarted/Finished
            // are already forwarded in real-time via forward_progress_event_broadcast.
            // Emitting them again would cause duplicate events with conflicting IDs.
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
            broadcast_event(&broadcast_tx, &turn_end);
            let run_msgs: Vec<serde_json::Value> =
                messages[before_len..].iter().map(message_to_json).collect();
            broadcast_event(&broadcast_tx, &AgentEvent::AgentEnd { messages: run_msgs });
            PromptOutcome::Success
        }
        Some(Err(e)) => {
            broadcast_event(
                &broadcast_tx,
                &AgentEvent::err(None, "agent_error", format!("{e}")),
            );
            PromptOutcome::Error
        }
    }
}

fn broadcast_event(tx: &tokio::sync::broadcast::Sender<String>, event: &AgentEvent) {
    let mut line = event.to_json_line();
    line.push('\n');
    let _ = tx.send(line);
}

/// Arguments for [`run_with_token_drain_broadcast`].
struct TokenDrainBroadcastArgs<'a> {
    broadcast_tx: &'a tokio::sync::broadcast::Sender<String>,
    agent: &'a mut AgentLoopImpl,
    messages: &'a mut Vec<Message>,
    progress_rx: &'a mut tokio::sync::mpsc::Receiver<crate::domain::agent::AgentProgressEvent>,
    cancel_rx: tokio::sync::oneshot::Receiver<()>,
    /// Subagent notification receiver — drained alongside tokens (#534).
    notification_rx:
        &'a mut Option<crate::infrastructure::tools::subagent_registry::NotificationRx>,
    /// Subagent registry for building state-changed events (#534).
    subagent_registry:
        &'a Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
}

/// Result of run_with_token_drain_broadcast, including collected notification messages (#534).
struct TokenDrainResult {
    result: Option<Result<crate::domain::agent::AgentResult, crate::domain::error::DomainError>>,
    /// Notifications collected during prompt execution, to be enqueued as pending.
    notifications:
        Vec<crate::infrastructure::tools::subagent_registry::SequencedSubagentNotification>,
}

async fn run_with_token_drain_broadcast(args: TokenDrainBroadcastArgs<'_>) -> TokenDrainResult {
    use crate::domain::agent::AgentLoop;

    let TokenDrainBroadcastArgs {
        broadcast_tx,
        agent,
        messages,
        progress_rx,
        cancel_rx,
        notification_rx,
        subagent_registry,
    } = args;

    tokio::pin!(cancel_rx);
    let mut process_fut = agent.process(messages);
    let mut notifications = Vec::new();

    let result = loop {
        // Build a future that drains notification_rx if present (#534).
        // This ensures SubagentStateChanged events propagate to TUI clients
        // in real-time during prompt execution, not just between prompts.
        let notif_recv = async {
            if let Some(rx) = notification_rx.as_mut() {
                rx.recv().await
            } else {
                std::future::pending().await
            }
        };

        tokio::select! {
            biased;
            _ = &mut cancel_rx => break None,
            Some(ev) = progress_rx.recv() => {
                forward_progress_event_broadcast(ev, broadcast_tx);
            }
            Some(notif) = notif_recv => {
                // Broadcast state-changed event to TUI AND collect message for LLM
                // injection — unless a manual `await` already consumed this
                // completion (auto-await dedupe), in which case the note is
                // suppressed and only the panel update fires.
                if forward_notification_broadcast(notif.clone(), broadcast_tx, subagent_registry) {
                    notifications.push(notif);
                }
            }
            result = &mut process_fut => {
                while let Ok(ev) = progress_rx.try_recv() {
                    forward_progress_event_broadcast(ev, broadcast_tx);
                }
                if let Some(rx) = notification_rx.as_mut() {
                    while let Ok(notif) = rx.try_recv() {
                        if forward_notification_broadcast(
                            notif.clone(),
                            broadcast_tx,
                            subagent_registry,
                        ) {
                            notifications.push(notif);
                        }
                    }
                }
                break Some(result);
            }
        }
    };

    TokenDrainResult {
        result,
        notifications,
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
    notif: crate::infrastructure::tools::subagent_registry::SequencedSubagentNotification,
    broadcast_tx: &tokio::sync::broadcast::Sender<String>,
    subagent_registry: &Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
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
        broadcast_event(broadcast_tx, &ev);
    }
    // Build full subagent info list from registry for the state-changed event.
    let list = protocol::build_subagent_info_list(subagent_registry);
    let ev = AgentEvent::SubagentStateChanged { subagents: list };
    broadcast_event(broadcast_tx, &ev);
    !suppress
}

fn forward_progress_event_broadcast(
    ev: crate::domain::agent::AgentProgressEvent,
    tx: &tokio::sync::broadcast::Sender<String>,
) {
    use crate::domain::agent::AgentProgressEvent;
    match ev {
        AgentProgressEvent::Token(t) => {
            broadcast_event(tx, &AgentEvent::Token { token: t });
        }
        AgentProgressEvent::ToolStarted {
            tool_call_id,
            name,
            arguments,
        } => {
            let args: serde_json::Value = serde_json::from_str(&arguments).unwrap_or_default();
            broadcast_event(
                tx,
                &AgentEvent::ToolExecutionStart {
                    tool_call_id,
                    tool_name: name,
                    args,
                },
            );
        }
        AgentProgressEvent::ToolFinished {
            tool_call_id,
            name,
            result_content,
            is_error,
            ..
        } => {
            broadcast_event(
                tx,
                &AgentEvent::ToolExecutionEnd {
                    tool_call_id,
                    tool_name: name,
                    result: crate::interface::cli::protocol::ToolResultContent {
                        content: vec![serde_json::json!({"type":"text","text": result_content})],
                    },
                    is_error,
                },
            );
        }
        AgentProgressEvent::TurnCompleted { messages } => {
            // Stream this turn's output on the agent's own broadcast stream. A
            // sub-agent emits this with an empty agent_id; the parent's monitor
            // re-stamps it with the child id before forwarding to the TUI (#797).
            let json: Vec<serde_json::Value> = messages.iter().map(message_to_json).collect();
            broadcast_event(
                tx,
                &AgentEvent::SubagentMessagesAppended {
                    agent_id: String::new(),
                    messages: json,
                },
            );
        }
        _ => {}
    }
}

/// Parsed representation of a client-sent `tool_result` command, ready
/// for `handle_tool_result` to consume.
pub(super) struct ParsedToolResult {
    pub(super) tool_call_id: String,
    pub(super) content: String,
    pub(super) is_error: bool,
}

/// Intercept a raw client line that carries a `tool_result` so it can
/// be resolved inline against `client_tool_registry`, bypassing the
/// (blocked-on-prompt) main dispatch loop.  Returns `None` when the
/// line isn't a tool_result, in which case the caller forwards it to
/// the dispatcher via the normal channel.
///
/// Implementation notes:
///
///  * **Cheap gate first.**  Every line the reader observes flows
///    through here, and the vast majority aren't tool_results.  A
///    cheap `contains` check short-circuits the full JSON parse for
///    the common case (prompts, register_tools, set_model, …).  The
///    needle is tight enough to avoid false positives on, say, a
///    prompt that literally discusses tool results.
///
///  * **Canonical parser reuse.**  Once the gate passes, we defer to
///    the same `parse_line` → `AgentCommand` path used by the main
///    dispatcher.  If `AgentCommand::ToolResult` ever gains a field,
///    the intercept path picks it up automatically and stays in sync
///    with the rest of the protocol surface — no hand-rolled JSON
///    extraction to drift.
pub(super) fn try_intercept_tool_result(line: &str) -> Option<ParsedToolResult> {
    if !line.contains(r#""tool_result""#) {
        return None;
    }
    match uds::parse_line(line) {
        uds::LineResult::Command(protocol::AgentCommand::ToolResult {
            tool_call_id,
            content,
            is_error,
        }) => Some(ParsedToolResult {
            tool_call_id,
            content,
            is_error,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent::AgentProgressEvent;
    use crate::domain::message::Message;

    #[test]
    fn turn_completed_broadcasts_subagent_messages_appended() {
        let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(8);
        forward_progress_event_broadcast(
            AgentProgressEvent::TurnCompleted {
                messages: vec![
                    Message::assistant("hello from turn", vec![]),
                    Message::tool("call-1", "tool body"),
                ]
                .into(),
            },
            &tx,
        );
        let line = rx.try_recv().expect("an event should be broadcast");
        assert!(line.contains("subagent_messages_appended"), "got: {line}");
        assert!(line.contains("\"agent_id\":\"\""), "got: {line}");
        assert!(line.contains("hello from turn"), "got: {line}");
        assert!(line.contains("tool body"), "got: {line}");
    }
}
