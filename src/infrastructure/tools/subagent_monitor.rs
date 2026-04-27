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
            entry.updated_at = Instant::now();
        }
        "agent_end" => {
            entry.status = SubagentStatus::Idle;
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
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        monitor_loop(&agent_id, &socket_path, &registry, notify_tx.as_ref()).await;
    })
}

/// Internal monitor loop: connect → read lines → apply events → detect close.
async fn monitor_loop(
    agent_id: &str,
    socket_path: &std::path::Path,
    registry: &SubagentRegistry,
    notify_tx: Option<&NotificationTx>,
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
                SubagentNotification::Exited {
                    agent_id: agent_id.to_string(),
                    sequence,
                },
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
                // Guard against oversized lines from misbehaving children.
                if line.len() > MAX_LINE_BYTES {
                    tracing::warn!(
                        agent = %agent_id,
                        len = line.len(),
                        "monitor: dropping oversized line"
                    );
                    continue;
                }
                // Only acquire the mutex lock for state-changing events.
                if STATE_CHANGING_EVENTS.iter().any(|pat| line.contains(pat)) {
                    // Parse once, use for both state update and notification.
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
                        let sequence = update_entry_next_sequence(registry, agent_id, |e| {
                            apply_event_parsed(e, &value)
                        });
                        notify_from_parsed(notify_tx, agent_id, sequence, &value);
                    }
                }
            }
            Ok(None) => {
                // EOF — child closed the connection.
                tracing::info!(agent = %agent_id, "monitor: child connection closed (EOF)");
                let sequence = update_entry_next_sequence(registry, agent_id, mark_exited);
                send_notification(
                    notify_tx,
                    SubagentNotification::Exited {
                        agent_id: agent_id.to_string(),
                        sequence,
                    },
                );
                return;
            }
            Err(e) => {
                tracing::warn!(agent = %agent_id, error = %e, "monitor: read error");
                let sequence = update_entry_next_sequence(registry, agent_id, mark_exited);
                send_notification(
                    notify_tx,
                    SubagentNotification::Exited {
                        agent_id: agent_id.to_string(),
                        sequence,
                    },
                );
                return;
            }
        }
    }
}

/// Check if a JSON-lines event should trigger a notification and send it.
/// Parses the line from string — use `notify_from_parsed` when you already have a Value.
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
                sequence,
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
                    sequence,
                    error: format!("tool '{}' returned error", tool_name),
                })
            } else {
                None
            }
        }
        _ => None,
    };
    if let Some(n) = notification {
        send_notification(Some(tx), n);
    }
}

/// Best-effort send of a notification (non-blocking, drops if channel is full).
fn send_notification(tx: Option<&NotificationTx>, notification: SubagentNotification) {
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
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_entry() -> SubagentEntry {
        SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0)
    }

    // --- apply_event: agent_start ---

    #[test]
    fn test_agent_start_sets_running() {
        let mut entry = test_entry();
        entry.status = SubagentStatus::Idle;
        apply_event(&mut entry, r#"{"type":"agent_start"}"#);
        assert_eq!(entry.status, SubagentStatus::Running);
    }

    #[test]
    fn test_agent_start_clears_last_error() {
        let mut entry = test_entry();
        entry.status = SubagentStatus::Error;
        entry.last_error = Some("old error".to_string());
        apply_event(&mut entry, r#"{"type":"agent_start"}"#);
        assert_eq!(entry.status, SubagentStatus::Running);
        assert!(entry.last_error.is_none());
    }

    // --- apply_event: agent_end ---

    #[test]
    fn test_agent_end_sets_idle() {
        let mut entry = test_entry();
        entry.status = SubagentStatus::Running;
        apply_event(&mut entry, r#"{"type":"agent_end","messages":[]}"#);
        assert_eq!(entry.status, SubagentStatus::Idle);
    }

    // --- apply_event: tool_execution_start ---

    #[test]
    fn test_tool_start_sets_running_and_last_tool() {
        let mut entry = test_entry();
        entry.status = SubagentStatus::Running;
        apply_event(
            &mut entry,
            r#"{"type":"tool_execution_start","toolCallId":"c1","toolName":"bash","args":{}}"#,
        );
        assert_eq!(entry.status, SubagentStatus::Running);
        assert_eq!(entry.last_tool.as_deref(), Some("bash"));
    }

    // --- apply_event: tool_execution_end ---

    #[test]
    fn test_tool_end_error_sets_error_and_last_error() {
        let mut entry = test_entry();
        entry.status = SubagentStatus::Running;
        apply_event(
            &mut entry,
            r#"{"type":"tool_execution_end","toolCallId":"c1","toolName":"edit","result":{"content":[]},"isError":true}"#,
        );
        assert_eq!(entry.status, SubagentStatus::Error);
        assert!(entry.last_error.as_ref().unwrap().contains("edit"));
    }

    #[test]
    fn test_tool_end_no_error_keeps_running() {
        let mut entry = test_entry();
        entry.status = SubagentStatus::Running;
        apply_event(
            &mut entry,
            r#"{"type":"tool_execution_end","toolCallId":"c1","toolName":"read","result":{"content":[]},"isError":false}"#,
        );
        assert_eq!(entry.status, SubagentStatus::Running);
    }

    #[test]
    fn test_tool_end_success_clears_last_error() {
        let mut entry = test_entry();
        entry.status = SubagentStatus::Running;
        entry.last_error = Some("previous error".to_string());
        apply_event(
            &mut entry,
            r#"{"type":"tool_execution_end","toolCallId":"c1","toolName":"read","result":{"content":[]},"isError":false}"#,
        );
        assert!(entry.last_error.is_none());
    }

    // --- apply_event: unknown / malformed ---

    #[test]
    fn test_unknown_event_ignored() {
        let mut entry = test_entry();
        entry.status = SubagentStatus::Idle;
        apply_event(&mut entry, r#"{"type":"token","token":"hello"}"#);
        assert_eq!(entry.status, SubagentStatus::Idle);
    }

    #[test]
    fn test_malformed_json_ignored() {
        let mut entry = test_entry();
        entry.status = SubagentStatus::Idle;
        apply_event(&mut entry, "not valid json");
        assert_eq!(entry.status, SubagentStatus::Idle);
    }

    // --- mark_exited ---

    #[test]
    fn test_mark_exited() {
        let mut entry = test_entry();
        entry.status = SubagentStatus::Running;
        mark_exited(&mut entry);
        assert_eq!(entry.status, SubagentStatus::Exited);
    }

    // --- truncate_string ---

    #[test]
    fn test_truncate_string_short() {
        assert_eq!(truncate_string("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_string_exact() {
        assert_eq!(truncate_string("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_string_long() {
        let result = truncate_string("hello world", 5);
        assert_eq!(result, "hello…");
    }

    // --- update_entry ---

    #[test]
    fn test_update_entry_modifies_registry() {
        let registry = super::super::subagent_registry::new_registry();
        registry.lock().unwrap().insert(
            "bot".to_string(),
            SubagentEntry::new(PathBuf::from("/tmp/bot.sock"), 0),
        );
        update_entry(&registry, "bot", |e| {
            e.status = SubagentStatus::Running;
        });
        let entries = registry.lock().unwrap();
        assert_eq!(entries["bot"].status, SubagentStatus::Running);
    }

    #[test]
    fn test_update_entry_missing_agent_is_noop() {
        let registry = super::super::subagent_registry::new_registry();
        // Should not panic
        update_entry(&registry, "nonexistent", |e| {
            e.status = SubagentStatus::Running;
        });
    }

    // --- pre-filter ---

    #[test]
    fn test_pre_filter_skips_token_events() {
        let mut entry = test_entry();
        entry.status = SubagentStatus::Idle;
        // Token event should be filtered out before JSON parse.
        apply_event(&mut entry, r#"{"type":"token","token":"hello"}"#);
        assert_eq!(entry.status, SubagentStatus::Idle);
    }

    // --- Sequence of events ---

    #[test]
    fn test_full_lifecycle() {
        let mut entry = test_entry();
        assert_eq!(entry.status, SubagentStatus::Starting);

        apply_event(&mut entry, r#"{"type":"agent_start"}"#);
        assert_eq!(entry.status, SubagentStatus::Running);

        apply_event(
            &mut entry,
            r#"{"type":"tool_execution_start","toolCallId":"c1","toolName":"bash","args":{}}"#,
        );
        assert_eq!(entry.status, SubagentStatus::Running);
        assert_eq!(entry.last_tool.as_deref(), Some("bash"));

        apply_event(
            &mut entry,
            r#"{"type":"tool_execution_end","toolCallId":"c1","toolName":"bash","result":{"content":[]},"isError":false}"#,
        );
        assert_eq!(entry.status, SubagentStatus::Running);

        apply_event(&mut entry, r#"{"type":"agent_end","messages":[]}"#);
        assert_eq!(entry.status, SubagentStatus::Idle);

        mark_exited(&mut entry);
        assert_eq!(entry.status, SubagentStatus::Exited);
    }

    // --- maybe_notify (#523) ---

    #[tokio::test]
    async fn test_notify_on_agent_end() {
        let (tx, mut rx) = super::super::subagent_registry::new_notification_channel();
        let line = r#"{"type":"agent_end","messages":[{"role":"assistant","content":"Done"}]}"#;
        maybe_notify(Some(&tx), "worker", line);
        let notif = rx.try_recv().unwrap();
        match notif {
            SubagentNotification::Completed {
                agent_id, summary, ..
            } => {
                assert_eq!(agent_id, "worker");
                assert_eq!(summary, "Done");
            }
            _ => panic!("expected Completed"),
        }
    }

    #[tokio::test]
    async fn test_notify_on_tool_error() {
        let (tx, mut rx) = super::super::subagent_registry::new_notification_channel();
        let line = r#"{"type":"tool_execution_end","toolCallId":"c1","toolName":"bash","result":{"content":[]},"isError":true}"#;
        maybe_notify(Some(&tx), "worker", line);
        let notif = rx.try_recv().unwrap();
        match notif {
            SubagentNotification::Errored {
                agent_id, error, ..
            } => {
                assert_eq!(agent_id, "worker");
                assert!(error.contains("bash"));
            }
            _ => panic!("expected Errored"),
        }
    }

    #[tokio::test]
    async fn test_no_notify_on_agent_start() {
        let (tx, mut rx) = super::super::subagent_registry::new_notification_channel();
        let line = r#"{"type":"agent_start"}"#;
        maybe_notify(Some(&tx), "worker", line);
        assert!(rx.try_recv().is_err(), "no notification should be sent");
    }

    #[tokio::test]
    async fn test_no_notify_on_successful_tool_end() {
        let (tx, mut rx) = super::super::subagent_registry::new_notification_channel();
        let line = r#"{"type":"tool_execution_end","toolCallId":"c1","toolName":"bash","result":{"content":[]},"isError":false}"#;
        maybe_notify(Some(&tx), "worker", line);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_notify_none_tx_is_noop() {
        // Should not panic
        maybe_notify(None, "worker", r#"{"type":"agent_end","messages":[]}"#);
    }

    #[tokio::test]
    async fn test_send_notification_exited() {
        let (tx, mut rx) = super::super::subagent_registry::new_notification_channel();
        send_notification(
            Some(&tx),
            SubagentNotification::Exited {
                agent_id: "bot".to_string(),
                sequence: 1,
            },
        );
        let notif = rx.try_recv().unwrap();
        assert_eq!(
            notif,
            SubagentNotification::Exited {
                agent_id: "bot".to_string(),
                sequence: 1,
            }
        );
    }

    #[tokio::test]
    async fn test_maybe_notify_agent_end() {
        let (tx, mut rx) = super::super::subagent_registry::new_notification_channel();
        let line = r#"{"type":"agent_end","messages":[{"role":"assistant","content":"done"}]}"#;
        maybe_notify(Some(&tx), "worker", line);
        let notif = rx.try_recv().unwrap();
        match notif {
            SubagentNotification::Completed {
                agent_id, summary, ..
            } => {
                assert_eq!(agent_id, "worker");
                assert!(summary.contains("done"));
            }
            _ => panic!("expected Completed"),
        }
    }

    #[tokio::test]
    async fn test_maybe_notify_tool_error() {
        let (tx, mut rx) = super::super::subagent_registry::new_notification_channel();
        let line = r#"{"type":"tool_execution_end","toolName":"bash","isError":true}"#;
        maybe_notify(Some(&tx), "worker", line);
        let notif = rx.try_recv().unwrap();
        match notif {
            SubagentNotification::Errored {
                agent_id, error, ..
            } => {
                assert_eq!(agent_id, "worker");
                assert!(error.contains("bash"));
            }
            _ => panic!("expected Errored"),
        }
    }

    #[tokio::test]
    async fn test_maybe_notify_tool_success_no_notification() {
        let (tx, mut rx) = super::super::subagent_registry::new_notification_channel();
        let line = r#"{"type":"tool_execution_end","toolName":"bash","isError":false}"#;
        maybe_notify(Some(&tx), "worker", line);
        assert!(rx.try_recv().is_err()); // No notification for success
    }

    #[test]
    fn test_maybe_notify_none_tx_is_noop() {
        let line = r#"{"type":"agent_end","messages":[]}"#;
        maybe_notify(None, "worker", line); // should not panic
    }

    #[test]
    fn test_maybe_notify_invalid_json_is_noop() {
        let (tx, mut rx) = super::super::subagent_registry::new_notification_channel();
        maybe_notify(Some(&tx), "worker", "not json");
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_maybe_notify_non_state_event_is_noop() {
        let (tx, mut rx) = super::super::subagent_registry::new_notification_channel();
        let line = r#"{"type":"token","token":"hello"}"#;
        maybe_notify(Some(&tx), "worker", line);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_notify_from_parsed_unknown_event_is_noop() {
        let (tx, mut rx) = super::super::subagent_registry::new_notification_channel();
        let value = serde_json::json!({"type": "token", "token": "hi"});
        notify_from_parsed(Some(&tx), "worker", 1, &value);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_notify_from_parsed_no_type_is_noop() {
        let (tx, mut rx) = super::super::subagent_registry::new_notification_channel();
        let value = serde_json::json!({"data": "something"});
        notify_from_parsed(Some(&tx), "worker", 1, &value);
        assert!(rx.try_recv().is_err());
    }
}
