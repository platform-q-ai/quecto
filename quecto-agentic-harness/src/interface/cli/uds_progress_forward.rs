use super::uds_cancel::EventSink;
use crate::domain::agents::agent::AgentProgressEvent;
use crate::interface::cli::protocol::{AgentEvent, ToolResultContent};

pub(crate) async fn forward_event(ev: AgentProgressEvent, sink: &mut EventSink<'_>) {
    match ev {
        AgentProgressEvent::Token(token) => sink.emit(&AgentEvent::Token { token }).await,
        AgentProgressEvent::ThinkingDelta(text) => sink.emit(&AgentEvent::Thinking { text }).await,
        AgentProgressEvent::ToolStarted {
            tool_call_id,
            name,
            arguments,
        } => {
            sink.emit(&AgentEvent::ToolExecutionStart {
                tool_call_id,
                tool_name: name,
                args: serde_json::from_str(&arguments)
                    .unwrap_or(serde_json::Value::String(arguments)),
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
            emit_tool_end(sink, tool_call_id, name, result_content, is_error).await;
        }
        AgentProgressEvent::TurnCompleted { messages } => {
            let message_refs: Vec<String> = messages.iter().map(|m| m.id().to_string()).collect();
            sink.emit(&AgentEvent::SubagentMessagesAppended {
                agent_id: String::new(),
                messages: vec![],
                message_refs,
            })
            .await;
        }
        AgentProgressEvent::ToolCatalogueChanged {
            changed_tools,
            before,
            after,
            reason,
        } => {
            sink.emit(&AgentEvent::ToolCatalogueChanged {
                changed_tools,
                before: before.into_iter().map(to_json).collect(),
                after: after.into_iter().map(to_json).collect(),
                reason,
            })
            .await;
        }
        AgentProgressEvent::ToolPolicyChanged {
            reconciliation,
            reason,
        } => {
            let correlation_id = reconciliation.correlation_id.clone();
            sink.emit(&AgentEvent::ToolPolicyChanged {
                changed_tools: reconciliation
                    .results
                    .iter()
                    .map(|r| r.name.clone())
                    .collect(),
                results: reconciliation.results.into_iter().map(to_json).collect(),
                apply_mode: match reconciliation.mode {
                    crate::domain::tool::ToolPolicyApplyMode::ImmediateIfIdle => {
                        "immediateIfIdle".to_string()
                    }
                    crate::domain::tool::ToolPolicyApplyMode::AtNextTurnBoundary => {
                        "atNextTurnBoundary".to_string()
                    }
                },
                reason,
                correlation_id,
            })
            .await;
        }
        AgentProgressEvent::RequestCompleted(completed) => {
            sink.emit(&request_completed(completed)).await;
        }
        _ => {}
    }
}

/// The `request_completed` event of one ended provider request (#2436).
fn request_completed(
    completed: crate::domain::inference::events::request_completion::RequestCompleted,
) -> AgentEvent {
    AgentEvent::RequestCompleted {
        model: completed.model,
        provider: completed.provider,
        input_tokens: completed.spend.map(|spend| spend.input_tokens),
        cached_tokens: completed.spend.and_then(|spend| spend.cached_tokens),
        cache_write_tokens: completed.spend.and_then(|spend| spend.cache_write_tokens),
        output_tokens: completed.spend.map(|spend| spend.output_tokens),
        duration_ms: completed.duration_ms,
        queued_ms: completed.queued_ms,
        outcome: completed.outcome,
        request_index: completed.request_index,
        attempt: completed.attempt,
    }
}

/// Forward the requests that ended as a cancelled turn was dropped (#2436):
/// its progress drain has stopped, so they are still queued. Only those:
/// what else the dropped turn queued is presentation it no longer shows.
pub(crate) async fn forward_settled_requests(
    progress: &mut tokio::sync::mpsc::Receiver<AgentProgressEvent>,
    sink: &mut EventSink<'_>,
) {
    while let Ok(event) = progress.try_recv() {
        if let AgentProgressEvent::RequestCompleted(completed) = event {
            sink.emit(&request_completed(completed)).await;
        }
    }
}

async fn emit_tool_end(
    sink: &mut EventSink<'_>,
    tool_call_id: String,
    tool_name: String,
    result_content: String,
    is_error: bool,
) {
    sink.emit(&AgentEvent::ToolExecutionEnd {
        tool_call_id,
        tool_name,
        result: ToolResultContent {
            content: vec![serde_json::json!({"type":"text","text": result_content})],
        },
        is_error,
    })
    .await;
}

fn to_json<T: serde::Serialize>(value: T) -> serde_json::Value {
    serde_json::to_value(value).unwrap_or_default()
}

#[cfg(test)]
#[path = "uds_progress_forward_tests.rs"]
mod tests;
