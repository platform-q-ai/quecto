use super::uds_cancel::EventSink;
use crate::domain::agent::AgentProgressEvent;
use crate::interface::cli::protocol::{AgentEvent, ToolResultContent};

pub(crate) async fn forward_event(ev: AgentProgressEvent, sink: &mut EventSink<'_>) {
    match ev {
        AgentProgressEvent::Token(token) => sink.emit(&AgentEvent::Token { token }).await,
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
            sink.emit(&AgentEvent::ToolPolicyChanged {
                changed_tools: reconciliation
                    .results
                    .iter()
                    .map(|r| r.name.clone())
                    .collect(),
                results: reconciliation.results.into_iter().map(to_json).collect(),
                child_propagation: reconciliation
                    .child_propagation
                    .into_iter()
                    .map(to_json)
                    .collect(),
                apply_mode: match reconciliation.mode {
                    crate::domain::tool::ToolPolicyApplyMode::ImmediateIfIdle => {
                        "immediateIfIdle".to_string()
                    }
                    crate::domain::tool::ToolPolicyApplyMode::AtNextTurnBoundary => {
                        "atNextTurnBoundary".to_string()
                    }
                },
                reason,
            })
            .await;
        }
        _ => {}
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
