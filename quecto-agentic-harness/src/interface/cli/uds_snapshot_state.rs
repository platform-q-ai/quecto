use super::*;

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
    state.sync = 1;
    let ev = AgentEvent::ok(
        None,
        "get_state",
        Some(crate::interface::cli::uds_state_projection::slim_state_projection(&state)),
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
    state.sync = 1;
    if let Some(ws) = workflow_state {
        let engine = ws.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
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
    let data = crate::interface::cli::uds_state_projection::slim_state_projection(&state);
    let ev = AgentEvent::ok(None, "get_state", Some(data));
    let mut line = ev.to_json_line();
    line.push('\n');
    line
}
