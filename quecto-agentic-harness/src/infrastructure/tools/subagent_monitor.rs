use super::subagent_lifecycle::{SubagentLifecycleEvent, apply_lifecycle_event};
pub use super::subagent_monitor_merge::{
    forward_child_state_changed, merge_and_forward_state_changed,
};
#[cfg(test)]
pub use super::subagent_monitor_registry::update_entry;
pub use super::subagent_monitor_registry::update_entry_next_sequence;
use super::subagent_monitor_stall::{
    classify_workflow_idle_stall, retry_pending_stalls, take_completion_armed,
};
use super::subagent_monitor_truncate::truncate_string;
#[cfg(test)]
pub use super::subagent_registry::SubagentStatus;
use super::subagent_registry::{
    NotificationTx, SubagentEntry, SubagentNotification, SubagentRegistry,
};
use std::time::Instant;
const MAX_EVENT_PAYLOAD_BYTES: usize = quecto_line_io::PROTOCOL_LINE_CAP_BYTES;
const MAX_STORED_STRING: usize = 256;
const STATE_CHANGING_EVENTS: &[&str] = &[
    "\"type\":\"agent_start\"",
    "\"type\":\"agent_end\"",
    "\"type\":\"tool_execution_start\"",
    "\"type\":\"tool_execution_end\"",
    "\"type\":\"workflow_state\"",
    "\"type\":\"workflow_idle\"",
    "\"command\":\"agent_error\"",
];
/// (via `mark_exited`).
pub fn apply_event(entry: &mut SubagentEntry, line: &str) {
    if !STATE_CHANGING_EVENTS.iter().any(|pat| line.contains(pat)) {
        return;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        return;
    };
    apply_event_parsed(entry, &value);
}

/// Apply a pre-parsed JSON event to a SubagentEntry.
/// This avoids a second parse when the caller already has the Value.
pub fn apply_event_parsed(entry: &mut SubagentEntry, value: &serde_json::Value) {
    let event_type = match value.get("type").and_then(|v| v.as_str()) {
        Some(t) => t,
        None => return,
    };
    match event_type {
        "agent_start" => {
            entry.status =
                apply_lifecycle_event(&mut entry.lifecycle, SubagentLifecycleEvent::RunStarted);
            entry.last_error = None;
            entry.run_error = None;
            // #1082 review round 2: a new run supersedes any stall alert
            // retained from the previous run — dropping it here prevents the
            // retry/backstop paths from attributing an old stall to this run.
            entry.pending_stall = None;
            // Re-arm the auto-await dedupe (#auto-await-idle): a new run means a
            // future terminal completion must notify again, even if a prior run's
            // completion was consumed by a manual `await`.
            entry.completion_consumed_by_await = false;
            entry.stalled_armed = true;
            entry.updated_at = Instant::now();
        }
        "agent_end" => {
            if entry.run_error.is_none() {
                entry.status =
                    apply_lifecycle_event(&mut entry.lifecycle, SubagentLifecycleEvent::RunEnded);
            }
            entry.updated_at = Instant::now();
        }
        "tool_execution_start" => {
            entry.status =
                apply_lifecycle_event(&mut entry.lifecycle, SubagentLifecycleEvent::ToolStarted);
            if let Some(tool_name) = value.get("toolName").and_then(|v| v.as_str()) {
                entry.last_tool = Some(truncate_string(tool_name, MAX_STORED_STRING));
            }
            entry.updated_at = Instant::now();
        }
        "tool_execution_end" => {
            // Recoverable tool errors are child-local. They should not poison
            // the parent-facing run status/error fields; only terminal
            // run-level failures (`agent_error`) do that.
            entry.updated_at = Instant::now();
        }
        "workflow_state" => {
            // Record the child's latest workflow snapshot on its registry entry
            // so the parent's SubagentInfo carries it (PRD Stage B / R-B3).
            let mode = value
                .get("mode")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let progress = value.get("progress");
            let done = progress
                .and_then(|p| p.get("done"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32;
            let total = progress
                .and_then(|p| p.get("total"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32;
            // Re-arm the terminal-completion latch whenever the workflow is NOT
            // in `complete` (i.e. still `active`): a workflow that transitions
            // back out of `complete` is a genuinely new run whose eventual
            // re-completion must notify again (#904). Staying in `complete`
            // across the `completion_nudge` follow-up turn leaves the latch
            // consumed, so that turn's `agent_end` stays silent.
            if mode != crate::domain::workflow::WorkflowMode::Complete.wire_str() {
                entry.completion_armed = true;
            }
            let workflow_progressed = entry.workflow.as_ref().is_none_or(|previous| {
                previous.mode != mode
                    || previous.steps_completed != done
                    || previous.steps_total != total
            });
            if workflow_progressed {
                entry.stalled_armed = true;
                // #1082 review round 2: fresh progress (or completion)
                // supersedes a stall retained under channel saturation — the
                // retained snapshot no longer describes the current state.
                entry.pending_stall = None;
            }
            entry.workflow = Some(super::subagent_registry::WorkflowSnapshot {
                mode,
                steps_completed: done,
                steps_total: total,
            });
            entry.updated_at = Instant::now();
        }
        "response" if value.get("command").and_then(|v| v.as_str()) == Some("agent_error") => {
            entry.status =
                apply_lifecycle_event(&mut entry.lifecycle, SubagentLifecycleEvent::RunFailed);
            let error = truncate_string(
                value
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("agent error"),
                MAX_STORED_STRING,
            );
            entry.last_error = Some(error.clone());
            entry.run_error = Some(error);
            // #1082 review round 2: the run's verdict is now Errored; drop any
            // retained stall so retry/backstop cannot also deliver Stalled.
            entry.pending_stall = None;
            entry.updated_at = Instant::now();
        }
        _ => {}
    }
}

/// Mark a SubagentEntry as Exited (connection closed or process reaped).
pub fn mark_exited(entry: &mut SubagentEntry) {
    entry.status =
        apply_lifecycle_event(&mut entry.lifecycle, SubagentLifecycleEvent::ProcessExited);
    // #1082 review round 3: exit supersedes a retained stall — the child is
    // gone, so an obsolete Stalled alert must not be claimable by the
    // capacity backstop or the event-driven retry after this point.
    entry.pending_stall = None;
    entry.updated_at = Instant::now();
}

/// Spawn a background monitor task that connects to a child agent's UDS socket
/// and reads the framed JSON event stream, updating the registry in real-time.
/// When `notify_tx` is `Some`, sends [`SubagentNotification`]s on the child's
/// completion/error/exit (#523). Returns an abortable `JoinHandle`.
pub fn spawn_monitor_task(
    agent_id: String,
    socket_path: std::path::PathBuf,
    registry: SubagentRegistry,
    notify_tx: Option<NotificationTx>,
    broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    parent_id: Option<String>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        monitor_loop(
            &agent_id,
            &socket_path,
            &registry,
            notify_tx.as_ref(),
            broadcast_tx.as_ref(),
            parent_id.as_deref(),
        )
        .await;
    })
}

fn notify_child_exited(
    registry: &SubagentRegistry,
    agent_id: &str,
    notify_tx: Option<&NotificationTx>,
) {
    let sequence = update_entry_next_sequence(registry, agent_id, mark_exited);
    let label = notification_display_label(registry, agent_id);
    let agent_uuid = notification_agent_uuid(registry, agent_id);
    send_notification(
        notify_tx,
        super::subagent_registry::SequencedSubagentNotification::new_for_agent(
            sequence,
            SubagentNotification::Exited { agent_id: label },
            agent_uuid,
        ),
    );
}

/// User-facing label for completion notes: prefer the entry's display name so
/// parents can `agent_cmd` with the same token they see in the note. Falls back
/// to the registry key (UUID) when the entry is already gone (#1378).
fn notification_display_label(registry: &SubagentRegistry, agent_id: &str) -> String {
    let entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    entries
        .get(agent_id)
        .map(|entry| entry.effective_display_name(agent_id).to_string())
        .unwrap_or_else(|| agent_id.to_string())
}

fn notification_agent_uuid(
    registry: &SubagentRegistry,
    agent_id: &str,
) -> crate::domain::ids::AgentUuid {
    let entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    entries
        .get(agent_id)
        .map(|entry| entry.agent_uuid.clone())
        .unwrap_or_else(|| crate::domain::ids::AgentUuid::new(agent_id))
}

/// Internal monitor loop: connect → read lines → apply events → detect close.
async fn monitor_loop(
    agent_id: &str,
    socket_path: &std::path::Path,
    registry: &SubagentRegistry,
    notify_tx: Option<&NotificationTx>,
    broadcast_tx: Option<&tokio::sync::broadcast::Sender<String>>,
    parent_id: Option<&str>,
) {
    use tokio::io::BufReader;

    // Retry connection with increasing backoff — the socket should already be
    // ready because spawn waits for it, but there's a tiny race window.
    let stream = match connect_with_retry(socket_path, 10).await {
        Some(s) => {
            update_entry_next_sequence(registry, agent_id, |entry| {
                entry.status = apply_lifecycle_event(
                    &mut entry.lifecycle,
                    SubagentLifecycleEvent::SocketConnected,
                );
            });
            s
        }
        None => {
            tracing::warn!(agent = %agent_id, "monitor: failed to connect to child socket");
            update_entry_next_sequence(registry, agent_id, |entry| {
                entry.status = apply_lifecycle_event(
                    &mut entry.lifecycle,
                    SubagentLifecycleEvent::SocketConnectFailed,
                );
            });
            notify_child_exited(registry, agent_id, notify_tx);
            return;
        }
    };

    // The monitor is otherwise listen-only, so announce framed mode with an
    // empty hello frame (ignored by the dispatch loop) — the child then
    // replies in length-prefixed frames (#1059 / ADR-0008 part 1). The write
    // half must stay open: dropping it would shut down the socket's write
    // direction and read as a client disconnect on the child.
    let (read_half, mut write_half) = tokio::io::split(stream);
    if let Err(e) = quecto_line_io::write_frame(
        &mut write_half,
        b"",
        quecto_line_io::PROTOCOL_FRAME_CAP_BYTES,
    )
    .await
    {
        tracing::warn!(agent = %agent_id, error = %e, "monitor: framed hello not delivered");
    }

    // Use a smaller BufReader capacity (1 KiB) since JSON events are
    // typically well under 1 KiB. Default 8 KiB is wasteful per child.
    let mut reader = BufReader::with_capacity(1024, read_half);

    // Reused across the child's whole event stream so each high-volume token
    // line does not allocate (and, on the framed branch, zero-initialize) a
    // fresh Vec — matching the sibling TUI/quecto-api hot readers migrated in
    // #1059.
    let mut buf = Vec::new();
    loop {
        match read_monitor_message(&mut reader, &mut buf, agent_id).await {
            MonitorRead::Message => {
                handle_monitor_line(
                    &String::from_utf8_lossy(&buf),
                    agent_id,
                    registry,
                    notify_tx,
                    broadcast_tx,
                    parent_id,
                );
            }
            MonitorRead::Skip => continue,
            MonitorRead::Closed => {
                crate::infrastructure::tools::container_script_cleanup::apply_container_inspect(
                    registry, agent_id,
                );
                notify_child_exited(registry, agent_id, notify_tx);
                return;
            }
        }
    }
}

/// Outcome of a single monitor read: a message (now in the caller's reusable
/// buffer), a recoverable skip (oversized frame rejected cleanly), or a
/// closed/broken connection.
enum MonitorRead {
    Message,
    Skip,
    Closed,
}

/// Read one framed-or-legacy message from the child into the reusable `buf`,
/// classifying EOF, oversized rejection (stream stays usable), and hard read
/// errors. Uses the buffer-reusing `_into` reader so a child's high-volume
/// token stream does not allocate per message.
async fn read_monitor_message<R>(reader: &mut R, buf: &mut Vec<u8>, agent_id: &str) -> MonitorRead
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    match quecto_line_io::read_frame_or_legacy_line_into(reader, buf, MAX_EVENT_PAYLOAD_BYTES).await
    {
        Ok(Some(_wire_mode)) => MonitorRead::Message,
        Ok(None) => {
            // EOF — child closed the connection.
            tracing::info!(agent = %agent_id, "monitor: child connection closed (EOF)");
            MonitorRead::Closed
        }
        Err(e @ quecto_line_io::FrameError::Oversized { .. }) => {
            // Over-cap message: rejected cleanly, stream stays framed.
            tracing::warn!(agent = %agent_id, "monitor: dropping oversized message: {e}");
            MonitorRead::Skip
        }
        Err(e) => {
            tracing::warn!(agent = %agent_id, error = %e, "monitor: read error");
            MonitorRead::Closed
        }
    }
}

/// Process one event line from a child: drop oversized lines, update the
/// registry entry + fire notifications for state-changing events, and forward
/// the child's workflow_state events onto the parent's stream (R-B2).
fn handle_monitor_line(
    line: &str,
    agent_id: &str,
    registry: &SubagentRegistry,
    notify_tx: Option<&NotificationTx>,
    broadcast_tx: Option<&tokio::sync::broadcast::Sender<String>>,
    parent_id: Option<&str>,
) {
    if line.len() > MAX_EVENT_PAYLOAD_BYTES {
        tracing::warn!(agent = %agent_id, len = line.len(), "monitor: dropping oversized line");
        return;
    }
    // Sub-agent stream events (per-turn messages and descendant state) are the
    // only lines forwarded onto the parent's stream from here. Gate both behind
    // one cheap substring fail-fast so high-volume `token` lines pay a single
    // scan instead of one per forward check (perf review).
    if let Some(tx) = broadcast_tx {
        if line.contains("\"subagent_") {
            // Per-turn message stream: forward re-stamped onto the parent's
            // stream so the TUI inspector updates turn-by-turn (#797). Not a
            // status change, so it bypasses the state-changing path below.
            if let Some(fwd) = forward_child_messages_appended(line, agent_id, parent_id) {
                // Re-stamping adds identity metadata and can cross the shared
                // frame cap. That is an invariant violation, not permission to
                // trim the child payload: reject the forwarded event whole.
                if fwd.len() > crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET {
                    tracing::warn!(
                        agent = %agent_id,
                        len = fwd.len(),
                        cap = crate::infrastructure::line_cap::EVENT_LINE_CAP_BYTES,
                        "monitor: dropping oversized forwarded event"
                    );
                    return;
                }
                let _ = tx.send(format!("{fwd}\n"));
                return;
            }
            // Descendant sub-agent state (a grandchild, or deeper) must reach the
            // root — and the TUI. MERGE descendants into the registry preserving
            // each entry's real identity, then re-broadcast the WHOLE tree so the
            // event keeps full-replace semantics and never evicts the root's own
            // children (#815). Not a status change for the immediate child, so it
            // returns early and bypasses the registry/notification path below.
            if let Some(fwd) = forward_child_state_changed(line, registry, agent_id) {
                // Already newline-terminated by build_state_changed_event (#1055).
                let _ = tx.send(fwd);
                return;
            }
        }
    }
    if !STATE_CHANGING_EVENTS.iter().any(|pat| line.contains(pat)) {
        return;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        return;
    };
    let foreign_workflow = value.get("type").and_then(|t| t.as_str()) == Some("workflow_state")
        && value
            .get("agent_id")
            .and_then(|v| v.as_str())
            .is_some_and(|a| !a.is_empty() && a != agent_id);
    if !foreign_workflow {
        apply_and_notify(registry, notify_tx, agent_id, &value);
    }
    if let Some(tx) = broadcast_tx {
        if !foreign_workflow && should_broadcast_state_changed_after_event(&value) {
            // Already newline-terminated by build_state_changed_event (#1055).
            let event = super::subagent_cascade::build_state_changed_event(registry);
            let _ = tx.send(event);
        }
        if let Some(fwd) = canonical_workflow_forward(&value, agent_id, parent_id) {
            if fwd.len() > crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET {
                tracing::warn!(agent = %agent_id, len = fwd.len(),
                    cap = crate::infrastructure::line_cap::EVENT_LINE_CAP_BYTES,
                    "monitor: dropping oversized forwarded workflow event");
                return;
            }
            let _ = tx.send(format!("{fwd}\n"));
        }
    }
}

/// Whether a child event changed the registry fields mirrored by
/// `subagent_state_changed` and should be pushed to TUI clients immediately
/// instead of waiting for a later polling rebuild (#839). Gated to terminal
/// transitions plus the FIRST running transition (`agent_start`, #866 — so a
/// newly-running child stays visible during a long first turn) — NOT the
/// high-frequency per-tool boundaries #839 removed (a running→idle transition is
/// carried by the broadcast `agent_end`, so no stale "running" persists).
pub fn should_broadcast_state_changed_after_event(value: &serde_json::Value) -> bool {
    match value.get("type").and_then(|v| v.as_str()) {
        Some("agent_start") => true,
        Some("agent_end") => true,
        Some("response") => value.get("command").and_then(|v| v.as_str()) == Some("agent_error"),
        _ => false,
    }
}

/// If `line` is a child's `workflow_state` event, re-stamp it with the child's
/// identity so it can be forwarded onto the parent's event stream (PRD Stage B
/// / R-B2): a parent/supervisor then sees descendant workflows without polling
/// each child socket. Returns the re-tagged JSON line, or `None` for any line
/// that is not a `workflow_state` event.
pub fn canonical_workflow_forward(
    value: &serde_json::Value,
    child_id: &str,
    parent_id: Option<&str>,
) -> Option<String> {
    if value.get("type").and_then(|t| t.as_str()) != Some("workflow_state") {
        return None;
    }
    // Re-build a canonical event from KNOWN fields (we do NOT pass through
    // arbitrary child-supplied keys). PRESERVE an existing descendant identity
    // when the event is already a forwarded grandchild workflow (#869c) — only
    // stamp the immediate child's id/parent when the event carries none — so a
    // grandchild's identity is not collapsed into the ancestor moving up the tree.
    let agent = value
        .get("agent_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(child_id);
    let parent = value
        .get("parent_id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| parent_id.map(str::to_string));
    let canonical = serde_json::json!({
        "type": "workflow_state",
        "agent_id": agent,
        "parent_id": parent,
        "mode": value.get("mode").cloned().unwrap_or(serde_json::Value::Null),
        "progress": value.get("progress").cloned().unwrap_or(serde_json::Value::Null),
    });
    serde_json::to_string(&canonical).ok()
}

/// Re-stamp a child's `subagent_messages_appended` with child/parent ids (#797)
/// and preserve messageRefs (#1060). Returns `None` if `value` is not that type.
#[rustfmt::skip]
pub fn canonical_messages_appended_forward(
    value: &serde_json::Value,
    child_id: &str,
    parent_id: Option<&str>,
) -> Option<String> {
    if value.get("type").and_then(|t| t.as_str()) != Some("subagent_messages_appended") {
        return None;
    }
    // #1060: prefer messageRefs; drop full messages when refs present.
    let empty = serde_json::json!([]);
    let refs = value.get("messageRefs").cloned().unwrap_or_else(|| empty.clone());
    let msgs = if refs.as_array().is_some_and(|a| !a.is_empty()) {
        empty
    } else {
        value.get("messages").cloned().unwrap_or(empty)
    };
    serde_json::to_string(&serde_json::json!({
        "type": "subagent_messages_appended", "agent_id": child_id,
        "parent_id": parent_id, "messages": msgs, "messageRefs": refs,
    })).ok()
}

/// Line-based wrapper around [`canonical_messages_appended_forward`].
pub fn forward_child_messages_appended(
    line: &str,
    child_id: &str,
    parent_id: Option<&str>,
) -> Option<String> {
    if !line.contains("\"type\":\"subagent_messages_appended\"") {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    canonical_messages_appended_forward(&value, child_id, parent_id)
}

/// Line-based wrapper around [`canonical_workflow_forward`].
pub fn forward_child_workflow_event(
    line: &str,
    child_id: &str,
    parent_id: Option<&str>,
) -> Option<String> {
    if !line.contains("\"type\":\"workflow_state\"") {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    canonical_workflow_forward(&value, child_id, parent_id)
}

/// Check if a JSON event should trigger a notification and send it.
/// Parses the line from string — use `notify_from_parsed` when you already have a Value.
#[cfg(test)]
pub fn maybe_notify(notify_tx: Option<&NotificationTx>, agent_id: &str, line: &str) {
    let Some(tx) = notify_tx else { return };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        return;
    };
    notify_from_parsed(
        Some(tx),
        agent_id,
        0,
        &value,
        None,
        crate::domain::ids::AgentUuid::new(agent_id),
    );
}

/// Apply a parsed event to the registry entry and fire any notification it
/// warrants, reading the entry's latest workflow mode AFTER the apply so the
/// terminal-completion decision (#904) sees up-to-date state.
fn apply_and_notify(
    registry: &SubagentRegistry,
    notify_tx: Option<&NotificationTx>,
    agent_id: &str,
    value: &serde_json::Value,
) {
    // #1082 review round 2: apply the incoming event BEFORE retrying retained
    // stalls. An `agent_error`, `agent_start`, workflow progress or terminal
    // completion invalidates a retained stall (see `apply_event_parsed`);
    // retrying first would publish the obsolete alert and only then learn it
    // was superseded.
    let sequence = update_entry_next_sequence(registry, agent_id, |e| apply_event_parsed(e, value));
    retry_pending_stalls(registry, notify_tx);
    let workflow_mode = entry_workflow_mode(registry, agent_id);
    // A terminal run error remains the observed outcome for this turn. `agent_end`
    // merely closes the turn and must not follow it with a success-like idle note.
    if value.get("type").and_then(|v| v.as_str()) == Some("agent_end")
        && registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(agent_id)
            .is_some_and(|entry| entry.run_error.is_some())
    {
        return;
    }
    // Latch a workflow-bound TERMINAL completion so the `completion_nudge`
    // "report and stop" follow-up turn — which also ends in `complete` mode —
    // does NOT fire a second completion note (#904). Only the FIRST terminal
    // `agent_end` per completion consumes the latch; a workflow re-entering
    // `active` re-arms it (see `apply_event_parsed`). Non-workflow agents
    // (`workflow_mode == None`) keep their unconditional turn-end note (#523).
    if value.get("type").and_then(|v| v.as_str()) == Some("agent_end")
        && workflow_mode.is_some()
        && agent_end_is_terminal(workflow_mode.as_deref())
        && !take_completion_armed(registry, agent_id)
    {
        return;
    }
    // Stall classification on `workflow_idle` lives with the other stall
    // logic in `subagent_monitor_stall` (#1082 review).
    if value.get("type").and_then(|v| v.as_str()) == Some("workflow_idle") {
        classify_workflow_idle_stall(registry, notify_tx, agent_id, sequence, value);
        return;
    }
    let note_label = notification_display_label(registry, agent_id);
    let agent_uuid = notification_agent_uuid(registry, agent_id);
    notify_from_parsed(
        notify_tx,
        &note_label,
        sequence,
        value,
        workflow_mode.as_deref(),
        agent_uuid,
    );
}

/// Read the latest workflow mode recorded on `agent_id`'s registry entry, if it
/// is workflow-bound. `None` means the agent has no workflow.
fn entry_workflow_mode(registry: &SubagentRegistry, agent_id: &str) -> Option<String> {
    let entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    entries
        .get(agent_id)
        .and_then(|e| e.workflow.as_ref().map(|w| w.mode.clone()))
}

/// Whether an `agent_end` represents a TRUE terminal completion (#904), given
/// the agent's latest workflow mode. A workflow-bound agent runs one turn per
/// step and ends each with its own `agent_end`; only the turn that leaves the
/// workflow in `complete` is terminal. A non-workflow agent (`None`) has no
/// auto-continue, so its turn-end is a logical completion.
pub fn agent_end_is_terminal(workflow_mode: Option<&str>) -> bool {
    match workflow_mode {
        Some(mode) => mode == crate::domain::workflow::WorkflowMode::Complete.wire_str(),
        None => true,
    }
}

/// Send a notification from a pre-parsed JSON value (avoids double parse).
/// `workflow_mode` is the agent's latest workflow mode (`None` if not
/// workflow-bound), used to gate the `agent_end` → `Completed` note to terminal
/// completion only (#904).
fn notify_from_parsed(
    notify_tx: Option<&NotificationTx>,
    agent_id: &str,
    sequence: u64,
    value: &serde_json::Value,
    workflow_mode: Option<&str>,
    agent_uuid: crate::domain::ids::AgentUuid,
) {
    let Some(tx) = notify_tx else { return };
    let event_type = match value.get("type").and_then(|v| v.as_str()) {
        Some(t) => t,
        None => return,
    };
    let notification = match event_type {
        // Only a TERMINAL agent_end fires a completion note: workflow `complete`,
        // or a non-workflow turn-end. A mid-workflow step-end auto-continues and
        // must stay silent so the parent isn't driven to re-narrate per step (#904).
        "agent_end" if agent_end_is_terminal(workflow_mode) => {
            // #1060: the completion NOTE is a fixed "inspect via get_messages"
            // pointer (see SubagentNotification::to_message) — no per-child
            // summary is derived or displayed, so agent_end.messages being
            // empty (refs-based) is fine here.
            // #1378: `agent_id` is already the display label (resolved by
            // `apply_and_notify`); lookups accept both display and UUID.
            Some(SubagentNotification::Completed {
                agent_id: agent_id.to_string(),
            })
        }
        "tool_execution_end" => None,
        "response" if value.get("command").and_then(|v| v.as_str()) == Some("agent_error") => {
            Some(SubagentNotification::Errored {
                agent_id: agent_id.to_string(),
                error: truncate_string(
                    value
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("agent error"),
                    MAX_STORED_STRING,
                ),
            })
        }
        _ => None,
    };
    if let Some(n) = notification {
        send_notification(
            Some(tx),
            super::subagent_registry::SequencedSubagentNotification::new_for_agent(
                sequence, n, agent_uuid,
            ),
        );
    }
}

/// Best-effort send of a notification (non-blocking, drops if channel is full).
fn send_notification(
    tx: Option<&NotificationTx>,
    notification: super::subagent_registry::SequencedSubagentNotification,
) {
    if let Some(tx) = tx {
        let _ = tx.try_send(notification);
    }
}

/// Connect to the UDS socket with retries and exponential backoff.
async fn connect_with_retry(
    socket_path: &std::path::Path,
    max_retries: u32,
) -> Option<tokio::net::UnixStream> {
    let mut delay_ms = 50u64;
    for _ in 0..max_retries {
        if let Ok(stream) = tokio::net::UnixStream::connect(socket_path).await {
            return Some(stream);
        }
        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        delay_ms = (delay_ms * 2).min(500); // Cap at 500ms
    }
    None
}

#[cfg(test)]
#[path = "subagent_monitor_lifecycle_tests.rs"]
mod lifecycle_tests;
#[cfg(test)]
#[path = "subagent_monitor_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "subagent_monitor_forward_tests.rs"]
mod forward_tests;

#[cfg(test)]
#[path = "tests/subagent_monitor_completion_tests.rs"]
mod completion_tests;

#[cfg(test)]
#[path = "tests/subagent_monitor_stall_race_tests.rs"]
mod stall_race_tests;

#[cfg(test)]
#[path = "tests/subagent_monitor_tool_error_tests.rs"]
mod tool_error_tests;

#[cfg(test)]
#[path = "subagent_monitor_bounded_read_tests.rs"]
mod bounded_read_tests;
#[cfg(test)]
#[path = "subagent_monitor_wave3_cov_tests.rs"]
mod wave3_cov_tests;
