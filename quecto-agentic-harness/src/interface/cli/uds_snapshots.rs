//! Busy-path inspector snapshots of one loop (#828, #880, #899): the
//! shared `SessionState`/stats/catalogue projections the reader tasks
//! serve while the dispatch loop is mid-turn, refreshed at every turn
//! boundary, and the connect-time snapshot lines a busy harness pushes.
//!
//! The conversation itself lives in the application-owned active session
//! (`ActiveSessionState`, #1971): this module publishes into it and reads
//! from it; `sync` is answered through the composed controller
//! (`uds_sync`, #1973) and `get_report` through the composed export
//! controller (#1974); the rewind's ledger reset is the rewind
//! transaction's (#1975); the session switch (identity, retention
//! namespace and transcript in one write) is the active session's own
//! `switch_to` (#1976).
use super::protocol::{AgentEvent, SessionState};
use super::uds::DispatchCtx;
use super::uds_session::{HISTORY_PAGE_SIZE, compute_session_stats_with_usage, history_page_json};
use super::uds_session_handles::SessionReadHandles;
use crate::application::sessions::dto::HistoryPage;
use crate::domain::conversation_view::user_visible_messages;

pub(crate) type StateSnapshot = std::sync::Arc<tokio::sync::RwLock<SessionState>>;

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
    refresh_tool_catalogue_snapshot(ctx).await;
}

pub(super) async fn refresh_conversation_snapshot(ctx: &DispatchCtx<'_>) {
    let mut session = ctx.sessions.active_session.write().await;
    let visible_messages = user_visible_messages(ctx.messages, ctx.system_prompt);
    let advance = session.publish(&visible_messages);
    drop(session);
    if advance.changed
        && let Some(tx) = ctx.broadcast_tx.as_ref()
    {
        let _ = tx.send(
            serde_json::json!({"type":"ledger_advanced","epoch":advance.epoch,"rev":advance.rev})
                .to_string()
                + "\n",
        );
    }
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
    let visible_message_count = user_visible_messages(ctx.messages, ctx.system_prompt).len();
    let state = ctx.session.state_snapshot(
        visible_message_count,
        workflow,
        ctx.agent.max_context_tokens(),
        ctx.agent.effort().map(|l| l.as_str().to_string()),
    );
    let mut snap = ctx.state_snapshot.write().await;
    *snap = state;
}

pub(super) async fn refresh_session_stats_snapshot(ctx: &DispatchCtx<'_>) {
    let visible_messages = user_visible_messages(ctx.messages, ctx.system_prompt);
    let stats = compute_session_stats_with_usage(
        ctx.session.session_key(),
        &visible_messages,
        ctx.session.usage_snapshot(),
        ctx.session.context_tokens(),
        ctx.agent.max_context_tokens(),
    );
    let mut snap = ctx.session_stats_snapshot.write().await;
    *snap = stats;
}

pub(super) async fn refresh_tool_catalogue_snapshot(ctx: &DispatchCtx<'_>) {
    let mut snap = ctx.tool_catalogue_snapshot.write().await;
    *snap = ctx
        .agent
        .tool_catalogue_entries()
        .into_iter()
        .map(|entry| serde_json::to_value(entry).unwrap_or_default())
        .collect();
}

/// Build the connect-time `get_messages` snapshot line a BUSY child pushes.
///
/// The `data.snapshot` marker tells callers the data may lag the in-flight turn
/// (a live dispatch-loop reply has no such marker) (#842). History uses the same
/// byte-bounded page shaping as the live query path so a BUSY child with one
/// oversized recent message still returns a recoverable summary (#1107).
pub(crate) fn build_get_messages_line(page: HistoryPage) -> String {
    let mut data = history_page_json(page);
    if let Some(obj) = data.as_object_mut() {
        if obj.get("before").is_some_and(serde_json::Value::is_null) {
            obj.remove("before");
        }
        obj.insert("snapshot".into(), serde_json::json!(true));
    }
    let mut line = AgentEvent::ok(None, "get_messages", Some(data)).to_json_line();
    debug_assert!(line.len() <= crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET);
    line.push('\n');
    line
}

/// Build the connect-time `get_subagents` snapshot line a BUSY child pushes.
pub(crate) fn build_get_subagents_line(
    registry: &Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
) -> String {
    let mut data = serde_json::to_value(
        super::protocol::build_compact_subagent_roster(registry, None).unwrap_or(
            super::protocol::CompactSubagentRoster {
                subagents: Vec::new(),
                sequence: 0,
                unchanged: None,
            },
        ),
    )
    .unwrap_or_default();
    if let Some(obj) = data.as_object_mut() {
        obj.insert("snapshot".to_string(), serde_json::json!(true));
    }
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

pub(crate) fn build_get_tool_catalogue_line(tools: &[serde_json::Value]) -> String {
    let data = serde_json::json!({
        "tools": tools,
        "snapshot": true,
    });
    let ev = AgentEvent::ok(None, "get_tool_catalogue", Some(data));
    let mut line = ev.to_json_line();
    line.push('\n');
    line
}

pub(crate) struct BusySnapshotSources<'a> {
    pub state: &'a StateSnapshot,
    pub session: &'a SessionReadHandles,
    pub session_stats: &'a SessionStatsSnapshot,
    pub tool_catalogue: &'a crate::interface::cli::uds_extensions::ToolCatalogueSnapshot,
    pub subagents: &'a Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
    pub workflow: &'a Option<crate::interface::shared::WorkflowStateHandle>,
    pub execution: &'a super::uds_execution_state::ExecutionStateHandle,
}

pub(crate) async fn busy_connect_snapshot_lines(sources: BusySnapshotSources<'_>) -> [String; 5] {
    let BusySnapshotSources {
        state: state_snapshot,
        session,
        session_stats: session_stats_snapshot,
        tool_catalogue: tool_catalogue_snapshot,
        subagents: subagent_registry,
        workflow: workflow_state,
        execution: execution_state,
    } = sources;
    let state_line = {
        // Release the async snapshot lock before taking either sync mutex.
        let live = state_snapshot.read().await.clone();
        build_busy_get_state_line(&live, workflow_state, execution_state)
    };
    let messages_line = build_get_messages_line(
        session
            .read_history
            .newest_live_page(HISTORY_PAGE_SIZE)
            .await,
    );
    let stats_line = {
        let stats = session_stats_snapshot.read().await;
        build_get_session_stats_line(&stats)
    };
    let extensions_line = {
        let tools = tool_catalogue_snapshot.read().await;
        build_get_tool_catalogue_line(&tools)
    };
    [
        state_line,
        messages_line,
        build_get_subagents_line(subagent_registry),
        stats_line,
        extensions_line,
    ]
}

pub(crate) fn build_busy_get_state_line(
    state: &SessionState,
    workflow_state: &Option<crate::interface::shared::WorkflowStateHandle>,
    execution_state: &super::uds_execution_state::ExecutionStateHandle,
) -> String {
    build_connect_get_state_line(state, workflow_state, execution_state, true)
}
pub(crate) fn build_connect_get_state_line(
    state: &SessionState,
    workflow_state: &Option<crate::interface::shared::WorkflowStateHandle>,
    execution_state: &super::uds_execution_state::ExecutionStateHandle,
    is_busy: bool,
) -> String {
    let mut live = state.clone();
    let workflow_revision = if let Some(workflow) = workflow_state {
        let engine = workflow
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let revision = engine.revision();
        live.workflow = Some(serde_json::to_value(engine.snapshot(true)).unwrap_or_default());
        revision
    } else {
        0
    };
    let mut execution = execution_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    live.generation = execution.observe_visible_revisions(live.generation, workflow_revision);
    live.message_count = execution.message_count();
    live.execution = Some(execution.snapshot());
    drop(execution);
    build_get_state_line_live(&live, &None, is_busy)
}

#[path = "uds_snapshot_state.rs"]
mod state_lines;
pub(crate) use state_lines::build_get_state_line_live;
#[cfg(test)]
pub(crate) use state_lines::{build_get_state_line, build_get_state_line_with_streaming};
