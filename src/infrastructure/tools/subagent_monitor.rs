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

use super::subagent_registry::{SubagentEntry, SubagentRegistry, SubagentStatus};

/// Apply a single JSON-line event to a SubagentEntry.
///
/// This is a pure function: it parses the event, updates the entry's status
/// fields, and returns. No I/O, no async.
///
/// State transitions follow the table from issue #522:
///
/// | Child Event                       | Status Transition                          |
/// |-----------------------------------|--------------------------------------------|
/// | `agent_start`                     | → `Running`                                |
/// | `agent_end`                       | → `Idle`                                   |
/// | `tool_execution_start`            | → `Running`, update `last_tool`            |
/// | `tool_execution_end` (is_error)   | → `Error`, set `last_error`                |
/// | `tool_execution_end` (!is_error)  | (no status change)                         |
/// | Connection closed / process exit  | → `Exited`  (via `mark_exited`)            |
pub fn apply_event(entry: &mut SubagentEntry, line: &str) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        // Malformed JSON — ignore silently (resilience).
        return;
    };

    let event_type = match value.get("type").and_then(|v| v.as_str()) {
        Some(t) => t,
        None => return, // No "type" field — ignore.
    };

    match event_type {
        "agent_start" => {
            entry.status = SubagentStatus::Running;
            entry.updated_at = Instant::now();
        }
        "agent_end" => {
            entry.status = SubagentStatus::Idle;
            entry.updated_at = Instant::now();
        }
        "tool_execution_start" => {
            entry.status = SubagentStatus::Running;
            if let Some(tool_name) = value.get("toolName").and_then(|v| v.as_str()) {
                entry.last_tool = Some(tool_name.to_string());
            }
            entry.updated_at = Instant::now();
        }
        "tool_execution_end" => {
            let is_error = value
                .get("isError")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if is_error {
                entry.status = SubagentStatus::Error;
                let tool_name = value
                    .get("toolName")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                entry.last_error = Some(format!("tool '{}' returned error", tool_name));
            }
            // If not an error, status stays as-is (Running).
            entry.updated_at = Instant::now();
        }
        // All other events (token, turn_start, turn_end, response, etc.) are
        // informational — no status change.
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
/// Returns a `JoinHandle` that can be aborted to stop the monitor.
pub fn spawn_monitor_task(
    agent_id: String,
    socket_path: std::path::PathBuf,
    registry: SubagentRegistry,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        monitor_loop(&agent_id, &socket_path, &registry).await;
    })
}

/// Internal monitor loop: connect → read lines → apply events → detect close.
async fn monitor_loop(agent_id: &str, socket_path: &std::path::Path, registry: &SubagentRegistry) {
    use tokio::io::{AsyncBufReadExt, BufReader};

    // Retry connection briefly — the socket should already be ready because
    // spawn waits for it, but there's a tiny race window.
    let stream = match connect_with_retry(socket_path, 5).await {
        Some(s) => s,
        None => {
            tracing::warn!(agent = %agent_id, "monitor: failed to connect to child socket");
            update_entry(registry, agent_id, mark_exited);
            return;
        }
    };

    let mut lines = BufReader::new(stream).lines();

    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                update_entry(registry, agent_id, |e| apply_event(e, &line));
            }
            Ok(None) => {
                // EOF — child closed the connection.
                tracing::info!(agent = %agent_id, "monitor: child connection closed (EOF)");
                update_entry(registry, agent_id, mark_exited);
                return;
            }
            Err(e) => {
                tracing::warn!(agent = %agent_id, error = %e, "monitor: read error");
                update_entry(registry, agent_id, mark_exited);
                return;
            }
        }
    }
}

/// Connect to the UDS socket with retries.
async fn connect_with_retry(
    socket_path: &std::path::Path,
    max_retries: u32,
) -> Option<tokio::net::UnixStream> {
    for _ in 0..max_retries {
        if let Ok(stream) = tokio::net::UnixStream::connect(socket_path).await {
            return Some(stream);
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    None
}

/// Update a single entry in the registry by name.
fn update_entry(registry: &SubagentRegistry, agent_id: &str, f: impl FnOnce(&mut SubagentEntry)) {
    let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(entry) = entries.get_mut(agent_id) {
        f(entry);
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
}
