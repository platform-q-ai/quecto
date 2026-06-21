// Persistent subagent monitor — live event stream from child agents (#522).
//
// Pure async monitor logic, no coupling to dispatch.
// The monitor task connects to a child's UDS socket, reads JSON-lines events,
// and updates the SubagentEntry in the SubagentRegistry with live status.
//
// NOTE: This module lives in the infrastructure layer and therefore MUST NOT
// import from `crate::interface` (architecture rule). Event JSON is parsed
// via `serde_json::Value` instead of deserializing into `AgentEvent`.

use std::time::Instant;

use super::subagent_registry::{
    NotificationTx, SubagentEntry, SubagentNotification, SubagentRegistry, SubagentStatus,
    extract_summary,
};

/// Maximum length for a single JSON-lines event (1 MiB).
/// Lines exceeding this are dropped to prevent OOM from misbehaving children.
const MAX_LINE_BYTES: usize = 1024 * 1024;

/// Maximum length for stored tool name / error strings (256 chars).
const MAX_STORED_STRING: usize = 256;

/// Event types that can cause state transitions.
/// Used for cheap pre-filtering before full JSON parse.
const STATE_CHANGING_EVENTS: &[&str] = &[
    "\"type\":\"agent_start\"",
    "\"type\":\"agent_end\"",
    "\"type\":\"tool_execution_start\"",
    "\"type\":\"tool_execution_end\"",
    "\"type\":\"workflow_state\"",
    "\"command\":\"agent_error\"",
];

/// Apply a single JSON-line event to a SubagentEntry.
///
/// This is a pure function: it parses the event, updates the entry's status
/// fields, and returns. No I/O, no async.
///
/// State transitions follow the table from issue #522:
///
/// | Child Event                       | Status Transition                          |
/// |-----------------------------------|--------------------------------------------|
/// | `agent_start`                     | → `Running`, clear `last_error`            |
/// | `agent_end`                       | → `Idle`                                   |
/// | `tool_execution_start`            | → `Running`, update `last_tool`            |
/// | `tool_execution_end` (is_error)   | → `Error`, set `last_error`                |
/// | `tool_execution_end` (!is_error)  | (no status change), clear `last_error`     |
/// | Connection closed / process exit  | → `Exited`  (via `mark_exited`)            |
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
            entry.status = SubagentStatus::Running;
            entry.last_error = None;
            entry.run_error = None;
            entry.updated_at = Instant::now();
        }
        "agent_end" => {
            if entry.run_error.is_none() {
                entry.status = SubagentStatus::Idle;
            }
            entry.updated_at = Instant::now();
        }
        "tool_execution_start" => {
            entry.status = SubagentStatus::Running;
            if let Some(tool_name) = value.get("toolName").and_then(|v| v.as_str()) {
                entry.last_tool = Some(truncate_string(tool_name, MAX_STORED_STRING));
            }
            entry.updated_at = Instant::now();
        }
        "tool_execution_end" => {
            let is_error = value
                .get("isError")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let tool_name = value
                .get("toolName")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            if is_error {
                entry.status = SubagentStatus::Error;
                entry.last_error = Some(truncate_string(
                    &format!("tool '{}' returned error", tool_name),
                    MAX_STORED_STRING,
                ));
            } else {
                entry.last_error = None;
            }
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
            entry.workflow = Some(super::subagent_registry::WorkflowSnapshot {
                mode,
                steps_completed: done,
                steps_total: total,
            });
            entry.updated_at = Instant::now();
        }
        "response" if value.get("command").and_then(|v| v.as_str()) == Some("agent_error") => {
            entry.status = SubagentStatus::Error;
            let error = truncate_string(
                value
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("agent error"),
                MAX_STORED_STRING,
            );
            entry.last_error = Some(error.clone());
            entry.run_error = Some(error);
            entry.updated_at = Instant::now();
        }
        _ => {}
    }
}

/// Mark a SubagentEntry as Exited (connection closed or process reaped).
pub fn mark_exited(entry: &mut SubagentEntry) {
    entry.status = SubagentStatus::Exited;
    entry.updated_at = Instant::now();
}

/// Spawn a background monitor task that connects to a child agent's UDS socket
/// and reads the JSON-lines event stream, updating the registry in real-time.
///
/// If `notify_tx` is `Some`, the monitor sends [`SubagentNotification`]s when
/// the child completes, errors, or exits (#523).
///
/// Returns a `JoinHandle` that can be aborted to stop the monitor.
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

/// Internal monitor loop: connect → read lines → apply events → detect close.
async fn monitor_loop(
    agent_id: &str,
    socket_path: &std::path::Path,
    registry: &SubagentRegistry,
    notify_tx: Option<&NotificationTx>,
    broadcast_tx: Option<&tokio::sync::broadcast::Sender<String>>,
    parent_id: Option<&str>,
) {
    use tokio::io::{AsyncBufReadExt, BufReader};

    // Retry connection with increasing backoff — the socket should already be
    // ready because spawn waits for it, but there's a tiny race window.
    let stream = match connect_with_retry(socket_path, 10).await {
        Some(s) => s,
        None => {
            tracing::warn!(agent = %agent_id, "monitor: failed to connect to child socket");
            let sequence = update_entry_next_sequence(registry, agent_id, mark_exited);
            send_notification(
                notify_tx,
                super::subagent_registry::SequencedSubagentNotification::new(
                    sequence,
                    SubagentNotification::Exited {
                        agent_id: agent_id.to_string(),
                    },
                ),
            );
            return;
        }
    };

    // Use a smaller BufReader capacity (1 KiB) since JSON-lines events are
    // typically well under 1 KiB. Default 8 KiB is wasteful per child.
    let mut lines = BufReader::with_capacity(1024, stream).lines();

    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                handle_monitor_line(
                    &line,
                    agent_id,
                    registry,
                    notify_tx,
                    broadcast_tx,
                    parent_id,
                );
            }
            Ok(None) => {
                // EOF — child closed the connection.
                tracing::info!(agent = %agent_id, "monitor: child connection closed (EOF)");
                let sequence = update_entry_next_sequence(registry, agent_id, mark_exited);
                send_notification(
                    notify_tx,
                    super::subagent_registry::SequencedSubagentNotification::new(
                        sequence,
                        SubagentNotification::Exited {
                            agent_id: agent_id.to_string(),
                        },
                    ),
                );
                return;
            }
            Err(e) => {
                tracing::warn!(agent = %agent_id, error = %e, "monitor: read error");
                let sequence = update_entry_next_sequence(registry, agent_id, mark_exited);
                send_notification(
                    notify_tx,
                    super::subagent_registry::SequencedSubagentNotification::new(
                        sequence,
                        SubagentNotification::Exited {
                            agent_id: agent_id.to_string(),
                        },
                    ),
                );
                return;
            }
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
    if line.len() > MAX_LINE_BYTES {
        tracing::warn!(agent = %agent_id, len = line.len(), "monitor: dropping oversized line");
        return;
    }
    // Cheap substring pre-filter: any line that isn't a tracked event type
    // (including high-volume `token` lines) is skipped before the JSON parse.
    if !STATE_CHANGING_EVENTS.iter().any(|pat| line.contains(pat)) {
        return;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        return;
    };
    // Parse once; reuse for the registry update, notification, and forwarding.
    let sequence =
        update_entry_next_sequence(registry, agent_id, |e| apply_event_parsed(e, &value));
    notify_from_parsed(notify_tx, agent_id, sequence, &value);
    // Forward workflow_state events onto the parent's stream as a canonical,
    // re-tagged event (R-B2).
    if let Some(tx) = broadcast_tx {
        if let Some(mut fwd) = canonical_workflow_forward(&value, agent_id, parent_id) {
            fwd.push('\n');
            let _ = tx.send(fwd);
        }
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
    // Re-build a canonical event from KNOWN fields with identity force-stamped.
    // We deliberately do NOT pass through arbitrary child-supplied keys onto the
    // parent's client stream.
    let canonical = serde_json::json!({
        "type": "workflow_state",
        "agent_id": child_id,
        "parent_id": parent_id,
        "mode": value.get("mode").cloned().unwrap_or(serde_json::Value::Null),
        "progress": value.get("progress").cloned().unwrap_or(serde_json::Value::Null),
    });
    serde_json::to_string(&canonical).ok()
}

/// Line-based wrapper around [`canonical_workflow_forward`]: cheap substring
/// pre-filter, then parse once. Returns `None` for non-`workflow_state` lines.
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

/// Check if a JSON-lines event should trigger a notification and send it.
/// Parses the line from string — use `notify_from_parsed` when you already have a Value.
#[cfg(test)]
pub fn maybe_notify(notify_tx: Option<&NotificationTx>, agent_id: &str, line: &str) {
    let Some(tx) = notify_tx else { return };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        return;
    };
    notify_from_parsed(Some(tx), agent_id, 0, &value);
}

/// Send a notification from a pre-parsed JSON value (avoids double parse).
fn notify_from_parsed(
    notify_tx: Option<&NotificationTx>,
    agent_id: &str,
    sequence: u64,
    value: &serde_json::Value,
) {
    let Some(tx) = notify_tx else { return };
    let event_type = match value.get("type").and_then(|v| v.as_str()) {
        Some(t) => t,
        None => return,
    };
    let notification = match event_type {
        "agent_end" => {
            // Use a reference to avoid cloning the entire messages array.
            let empty = serde_json::Value::Array(vec![]);
            let messages = value.get("messages").unwrap_or(&empty);
            let summary = extract_summary(messages);
            Some(SubagentNotification::Completed {
                agent_id: agent_id.to_string(),
                summary,
            })
        }
        "tool_execution_end" => {
            let is_error = value
                .get("isError")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if is_error {
                let tool_name = value
                    .get("toolName")
                    .and_then(|v| v.as_str())
                    .map(|s| truncate_string(s, MAX_STORED_STRING))
                    .unwrap_or_else(|| "unknown".to_string());
                Some(SubagentNotification::Errored {
                    agent_id: agent_id.to_string(),
                    error: format!("tool '{}' returned error", tool_name),
                })
            } else {
                None
            }
        }
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
            super::subagent_registry::SequencedSubagentNotification::new(sequence, n),
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
fn update_entry(registry: &SubagentRegistry, agent_id: &str, f: impl FnOnce(&mut SubagentEntry)) {
    let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(entry) = entries.get_mut(agent_id) {
        f(entry);
    }
}

/// Update an entry and allocate the next monotonic notification sequence.
fn update_entry_next_sequence(
    registry: &SubagentRegistry,
    agent_id: &str,
    f: impl FnOnce(&mut SubagentEntry),
) -> u64 {
    let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(entry) = entries.get_mut(agent_id) {
        f(entry);
        entry.notification_sequence = entry.notification_sequence.saturating_add(1);
        entry.notification_sequence
    } else {
        0
    }
}

/// Truncate a string to at most `max_len` characters, appending "…" if truncated.
fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let mut truncated = s[..max_len].to_string();
        truncated.push('…');
        truncated
    }
}

// ─── Unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "subagent_monitor_tests.rs"]
mod tests;
