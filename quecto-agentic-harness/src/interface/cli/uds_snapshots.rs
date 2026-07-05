use crate::domain::message::Message;

use serde_json::value::RawValue;

use super::protocol::{AgentEvent, SessionState};
use super::uds::DispatchCtx;
use super::uds_session::{MessageView, compute_session_stats_with_usage};

pub(crate) type StateSnapshot = std::sync::Arc<tokio::sync::RwLock<SessionState>>;
pub(crate) type ConversationSnapshot = std::sync::Arc<tokio::sync::RwLock<Vec<Message>>>;
pub(crate) type SessionStatsSnapshot =
    std::sync::Arc<tokio::sync::RwLock<crate::interface::cli::protocol::SessionStats>>;

/// Refresh every busy-child snapshot (state / conversation / session_stats /
/// extensions) at once. Called per INNER turn inside the drain/nudge loop so a
/// busy `get_state` mid-workflow tracks progress + message count step-by-step,
/// instead of being frozen at the pre-turn (often initial) view until the whole
/// dispatched command returns (#899). The `snapshot: true` staleness marker is
/// retained — a busy snapshot may still lag the in-flight turn by design, but it
/// must not lag by an entire workflow.
pub(super) async fn refresh_busy_snapshots(ctx: &DispatchCtx<'_>) {
    refresh_conversation_snapshot(ctx).await;
    refresh_state_snapshot(ctx).await;
    refresh_session_stats_snapshot(ctx).await;
    refresh_extension_snapshot(ctx).await;
}

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
pub(crate) const SNAPSHOT_MESSAGES_BUDGET_BYTES: usize = 1024 * 1024 - 4096;

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
    // Serialize each message EXACTLY ONCE into an owned `RawValue`; its `.get()`
    // length is used for byte-budgeting and the same bytes are re-emitted
    // verbatim in the final line (no second serialization, no Value tree) (#994).
    let mut raws: Vec<Box<RawValue>> = messages
        .iter()
        .map(|m| {
            serde_json::value::to_raw_value(&MessageView(m)).unwrap_or_else(|_| {
                RawValue::from_string("null".to_string()).expect("null literal")
            })
        })
        .collect();

    // Accumulate from the newest message backwards until the next (older) one
    // would breach the budget; `start` is the index of the oldest kept message.
    let mut total = 0usize;
    let mut start = raws.len();
    for (i, rv) in raws.iter().enumerate().rev() {
        let sz = rv.get().len() + 1; // +1 for the array separator
        if total + sz > SNAPSHOT_MESSAGES_BUDGET_BYTES {
            break;
        }
        total += sz;
        start = i;
    }
    let trimmed = start > 0;
    // `split_off` moves the kept tail out in place — no slice clone (#994).
    let kept = raws.split_off(start);

    let line_body = GetMessagesSnapshot::Response {
        command: "get_messages",
        success: true,
        data: GetMessagesData {
            messages: &kept,
            snapshot: true,
            trimmed,
        },
    };
    let mut line =
        serde_json::to_string(&line_body).expect("get_messages snapshot is always serializable");
    line.push('\n');
    line
}

/// Serializes byte-identically (modulo key order) to
/// `AgentEvent::ok(None, "get_messages", Some(data))`, but embeds the
/// pre-serialized message `RawValue`s directly so each message is serialized at
/// most once (#994).
#[derive(serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum GetMessagesSnapshot<'a> {
    Response {
        command: &'a str,
        success: bool,
        data: GetMessagesData<'a>,
    },
}

#[derive(serde::Serialize)]
struct GetMessagesData<'a> {
    messages: &'a [Box<RawValue>],
    snapshot: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    trimmed: bool,
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
    workflow_state: &Option<crate::interface::shared::WorkflowStateHandle>,
) -> [String; 5] {
    let state_line = {
        let snap = state_snapshot.read().await;
        // #914: overlay the LIVE workflow engine onto the (turn-boundary) frozen
        // snapshot so a busy `get_state` reports mid-turn step progress, not just
        // 0/N (pre-turn) or N/N (post-turn). The engine `Mutex` is independent of
        // the dispatch loop's `&mut messages`, so this is safe to read mid-turn.
        build_get_state_line_live(&snap, workflow_state, true)
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

#[cfg(test)]
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

/// Busy `get_state` line with the LIVE workflow engine overlaid onto the frozen
/// session snapshot (#914). The periodic `state_snapshot` only refreshes at turn
/// boundaries, but workflow steps are checked off mid-turn via the `workflow`
/// tool, so a busy `get_state` served from the frozen snapshot only ever shows
/// `0/N` (pre-turn) or `N/N` (post-turn). The engine is an `Arc<Mutex<…>>`
/// independent of the dispatch loop's `&mut messages`; we lock it briefly and
/// synchronously (no `.await` held) to read its current snapshot, mirroring how
/// `refresh_state_snapshot` serializes it. Automation flags are preserved from
/// the frozen snapshot (they come from workflow config, not the engine).
pub(crate) fn build_get_state_line_live(
    state: &SessionState,
    workflow_state: &Option<crate::interface::shared::WorkflowStateHandle>,
    is_streaming: bool,
) -> String {
    let mut state = state.clone();
    state.is_streaming = is_streaming;
    if let Some(ws) = workflow_state {
        if let Ok(engine) = ws.lock() {
            let mut live = serde_json::to_value(engine.snapshot(true)).unwrap_or_default();
            if let Some(auto) = state
                .workflow
                .as_ref()
                .and_then(|w| w.get("automation"))
                .cloned()
            {
                live["automation"] = auto;
            }
            state.workflow = Some(live);
        }
    }
    let ev = AgentEvent::ok(
        None,
        "get_state",
        Some(serde_json::to_value(state).unwrap_or_default()),
    );
    let mut line = ev.to_json_line();
    line.push('\n');
    line
}
