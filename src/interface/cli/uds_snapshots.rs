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

/// Byte budget for a connect-time `get_messages` snapshot line. Kept just under
/// the parent's per-line read cap (`SUBAGENT_RESPONSE_MAX_LINE_BYTES` = 1 MiB in
/// `subagent_registry`) with headroom for the response envelope, so an oversized
/// history is tailed to fit rather than making the parent's whole call error
/// ("line exceeded size limit") on a busy child (#842).
const SNAPSHOT_MESSAGES_BUDGET_BYTES: usize = 1024 * 1024 - 4096;

/// Build the connect-time `get_messages` snapshot line a BUSY child pushes.
///
/// The `data.snapshot` marker tells callers the data may lag the in-flight turn
/// (a live dispatch-loop reply has no such marker) (#842). When the serialized
/// history would exceed [`SNAPSHOT_MESSAGES_BUDGET_BYTES`], the OLDEST messages
/// are dropped so the most recent (the inspection target) still arrive, with
/// `data.trimmed` set — counted/tail readers slice this further on the parent
/// side, so a tail is exactly what they want.
pub(crate) fn build_get_messages_line(messages: &[Message]) -> String {
    let values: Vec<serde_json::Value> = messages.iter().map(message_to_json).collect();

    // Accumulate from the newest message backwards until the next (older) one
    // would breach the budget; `start` is the index of the oldest kept message.
    let mut total = 0usize;
    let mut start = values.len();
    for (i, v) in values.iter().enumerate().rev() {
        let sz = v.to_string().len() + 1; // +1 for the array separator
        if total + sz > SNAPSHOT_MESSAGES_BUDGET_BYTES {
            break;
        }
        total += sz;
        start = i;
    }
    let trimmed = start > 0;
    let msgs: Vec<serde_json::Value> = values[start..].to_vec();

    let mut data = serde_json::json!({ "messages": msgs, "snapshot": true });
    if trimmed {
        data["trimmed"] = serde_json::json!(true);
    }
    let ev = AgentEvent::ok(None, "get_messages", Some(data));
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
