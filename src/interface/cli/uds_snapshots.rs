use crate::domain::message::Message;

use super::protocol::{AgentEvent, SessionState};
use super::uds::DispatchCtx;
use super::uds_session::message_to_json;

pub(crate) type StateSnapshot = std::sync::Arc<tokio::sync::RwLock<SessionState>>;
pub(crate) type ConversationSnapshot = std::sync::Arc<tokio::sync::RwLock<Vec<Message>>>;

pub(super) async fn refresh_conversation_snapshot(ctx: &DispatchCtx<'_>) {
    let mut snap = ctx.conversation_snapshot.write().await;
    *snap = ctx.messages.clone();
}

pub(super) async fn refresh_state_snapshot(ctx: &DispatchCtx<'_>) {
    let workflow = ctx.workflow_state.as_ref().and_then(|ws| {
        ws.lock().ok().map(|engine| {
            let mut value = serde_json::to_value(engine.snapshot(true)).unwrap_or_default();
            if let Some(config) = &ctx.workflow_config {
                value["automation"] = serde_json::json!({
                    "autoContinue": config.auto_continue,
                    "completionNudge": config.completion_nudge,
                });
            }
            value
        })
    });
    let state =
        ctx.session
            .state_snapshot(ctx.messages.len(), workflow, ctx.agent.max_context_tokens());
    let mut snap = ctx.state_snapshot.write().await;
    *snap = state;
}

pub(crate) fn build_get_messages_line(messages: &[Message]) -> String {
    let msgs: Vec<serde_json::Value> = messages.iter().map(message_to_json).collect();
    let ev = AgentEvent::ok(
        None,
        "get_messages",
        Some(serde_json::json!({ "messages": msgs })),
    );
    let mut line = ev.to_json_line();
    line.push('\n');
    line
}

#[cfg(test)]
pub(crate) fn build_get_state_line(state: &SessionState) -> String {
    build_get_state_line_with_streaming(state, state.is_streaming)
}

pub(crate) fn build_get_state_line_with_streaming(
    state: &SessionState,
    is_streaming: bool,
) -> String {
    let mut state = state.clone();
    state.is_streaming = is_streaming;
    let ev = AgentEvent::ok(
        None,
        "get_state",
        Some(serde_json::to_value(state).unwrap_or_default()),
    );
    let mut line = ev.to_json_line();
    line.push('\n');
    line
}
