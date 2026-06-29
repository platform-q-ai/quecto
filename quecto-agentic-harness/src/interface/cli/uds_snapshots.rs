use crate::domain::message::Message;

use super::protocol::{AgentEvent, SessionState};
use super::uds::DispatchCtx;
use super::uds_session::{compute_session_stats_with_usage, message_to_json};

pub(crate) type StateSnapshot = std::sync::Arc<tokio::sync::RwLock<SessionState>>;
pub(crate) type ConversationSnapshot = std::sync::Arc<tokio::sync::RwLock<Vec<Message>>>;
pub(crate) type SessionStatsSnapshot =
    std::sync::Arc<tokio::sync::RwLock<crate::interface::cli::protocol::SessionStats>>;

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

pub(super) async fn refresh_session_stats_snapshot(ctx: &DispatchCtx<'_>) {
    let stats = compute_session_stats_with_usage(
        ctx.session_key,
        ctx.messages,
        ctx.session.usage_snapshot(),
        ctx.session.context_tokens(),
        ctx.agent.max_context_tokens(),
    );
    let mut snap = ctx.session_stats_snapshot.write().await;
    *snap = stats;
}

pub(super) async fn refresh_extension_snapshot(ctx: &DispatchCtx<'_>) {
    let mut snap = ctx.extension_snapshot.write().await;
    *snap = crate::interface::cli::uds_extensions::build_extension_list(ctx);
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
/// side, so a tail is exactly what they want. A single message that alone exceeds
/// the budget cannot be returned under the parent's read cap, so it is dropped
/// too (yielding an empty `trimmed` snapshot rather than erroring the call).
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

/// Build the connect-time `get_subagents` snapshot line a BUSY child pushes.
///
/// The `SubagentRegistry` is an `Arc<Mutex<…>>` independent of the dispatch
/// loop's exclusive `&mut messages` borrow, so a busy child can serve its
/// current registry view off the turn (#874). A `None` registry yields an empty
/// subagents list (matching [`build_subagent_info_list`]'s contract), not an
/// error. The `data.snapshot` marker tells callers the data may lag the
/// in-flight turn, consistent with the #842 snapshot markers.
pub(crate) fn build_get_subagents_line(
    registry: &Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
) -> String {
    let data = serde_json::json!({
        "subagents": serde_json::to_value(super::protocol::build_subagent_info_list(registry))
            .unwrap_or_default(),
        "snapshot": true,
    });
    let ev = AgentEvent::ok(None, "get_subagents", Some(data));
    let mut line = ev.to_json_line();
    line.push('\n');
    line
}

pub(crate) fn build_get_session_stats_line(
    stats: &crate::interface::cli::protocol::SessionStats,
) -> String {
    let mut data = serde_json::to_value(stats).unwrap_or_default();
    if let Some(obj) = data.as_object_mut() {
        obj.insert("snapshot".to_string(), serde_json::json!(true));
    }
    let ev = AgentEvent::ok(None, "get_session_stats", Some(data));
    let mut line = ev.to_json_line();
    line.push('\n');
    line
}

pub(crate) fn build_get_extensions_line(extensions: &[serde_json::Value]) -> String {
    let data = serde_json::json!({
        "extensions": extensions,
        "snapshot": true,
    });
    let ev = AgentEvent::ok(None, "get_extensions", Some(data));
    let mut line = ev.to_json_line();
    line.push('\n');
    line
}

pub(crate) async fn busy_connect_snapshot_lines(
    state_snapshot: &StateSnapshot,
    conversation_snapshot: &ConversationSnapshot,
    session_stats_snapshot: &SessionStatsSnapshot,
    extension_snapshot: &crate::interface::cli::uds_extensions::ExtensionSnapshot,
    subagent_registry: &Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
) -> [String; 5] {
    let state_line = {
        let snap = state_snapshot.read().await;
        build_get_state_line_with_streaming(&snap, true)
    };
    let messages_line = {
        let messages = conversation_snapshot.read().await;
        build_get_messages_line(&messages)
    };
    let stats_line = {
        let stats = session_stats_snapshot.read().await;
        build_get_session_stats_line(&stats)
    };
    let extensions_line = {
        let extensions = extension_snapshot.read().await;
        build_get_extensions_line(&extensions)
    };
    [
        state_line,
        messages_line,
        build_get_subagents_line(subagent_registry),
        stats_line,
        extensions_line,
    ]
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
