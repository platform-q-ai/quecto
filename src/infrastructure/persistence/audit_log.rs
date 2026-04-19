//! Append-only audit log writer.
//!
//! Writes one JSON line per event to `<base_dir>/audit/<session_key>.jsonl`.
//! Flushed on every write to survive crashes.

use std::path::{Path, PathBuf};

use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use crate::domain::audit::{AuditEnvelope, AuditEvent, AuditSink};
use crate::domain::error::DomainError;
use crate::infrastructure::persistence::filename::sanitize_session_key;

/// Append-only audit log handle for a single session.
///
/// Uses a raw `tokio::fs::File` (no `BufWriter`) because every `emit()` call
/// flushes immediately for crash durability — buffering would be negated.
pub struct AuditLog {
    writer: Mutex<tokio::fs::File>,
    session_key: String,
}

impl std::fmt::Debug for AuditLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditLog")
            .field("session_key", &self.session_key)
            .finish()
    }
}

impl AuditLog {
    /// Open (or create) the audit log file for the given session (async).
    ///
    /// Creates `<base_dir>/audit/` if it doesn't exist (lazy init).
    /// The file is opened in append mode so restarts continue the same log.
    pub async fn open(base_dir: &Path, session_key: &str) -> Result<Self, DomainError> {
        let audit_dir = base_dir.join("audit");
        tokio::fs::create_dir_all(&audit_dir)
            .await
            .map_err(|e| DomainError::Session(format!("failed to create audit dir: {e}")))?;

        let filename = format!("{}.jsonl", sanitize_session_key(session_key));
        let path = audit_dir.join(&filename);

        let file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .map_err(|e| DomainError::Session(format!("failed to open audit log: {e}")))?;

        Ok(Self {
            writer: Mutex::new(file),
            session_key: session_key.to_string(),
        })
    }

    /// Open (or create) the audit log file for the given session (sync).
    ///
    /// Same as [`Self::open`] but uses blocking I/O. Suitable for use in
    /// sync contexts (e.g. before the tokio runtime is entered).
    pub fn open_sync(base_dir: &Path, session_key: &str) -> Result<Self, DomainError> {
        let audit_dir = base_dir.join("audit");
        std::fs::create_dir_all(&audit_dir)
            .map_err(|e| DomainError::Session(format!("failed to create audit dir: {e}")))?;

        let filename = format!("{}.jsonl", sanitize_session_key(session_key));
        let path = audit_dir.join(&filename);

        let std_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| DomainError::Session(format!("failed to open audit log: {e}")))?;

        let tokio_file = tokio::fs::File::from_std(std_file);

        Ok(Self {
            writer: Mutex::new(tokio_file),
            session_key: session_key.to_string(),
        })
    }

    /// Emit a single audit event.
    ///
    /// Serialises with envelope fields, writes one JSONL line, and flushes.
    /// The flush is critical — the log must survive crashes.
    pub async fn emit(&self, turn: u32, event: AuditEvent) -> Result<(), DomainError> {
        let envelope = AuditEnvelope {
            ts: now_utc_iso8601(),
            session: self.session_key.clone(),
            turn,
            event,
        };

        let mut line =
            serde_json::to_string(&envelope).map_err(|e| DomainError::Other(e.to_string()))?;
        line.push('\n');

        // Write directly — no BufWriter since we need every line flushed for
        // crash durability. On Linux, append-mode writes of < PIPE_BUF (4096)
        // bytes are atomic, and a typical JSONL line is 200-500 bytes.
        //
        // `flush` after `write_all` is required even without BufWriter:
        // tokio::fs::File wraps std::fs::File on a blocking thread pool and
        // may hold a pending write across `await` points. Without the flush,
        // a drop of the File (or a process crash) can lose the last line —
        // exactly the case the contract test caught.
        let mut writer = self.writer.lock().await;
        writer
            .write_all(line.as_bytes())
            .await
            .map_err(|e| DomainError::Session(format!("audit log write failed: {e}")))?;
        writer
            .flush()
            .await
            .map_err(|e| DomainError::Session(format!("audit log flush failed: {e}")))?;

        Ok(())
    }

    /// Return the path to the audit log file for a given session key.
    ///
    /// Useful for tests and external consumers.
    pub fn file_path(base_dir: &Path, session_key: &str) -> PathBuf {
        let filename = format!("{}.jsonl", sanitize_session_key(session_key));
        base_dir.join("audit").join(filename)
    }
}

impl AuditSink for AuditLog {
    fn emit(
        &self,
        turn: u32,
        event: AuditEvent,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), DomainError>> + Send + '_>> {
        Box::pin(AuditLog::emit(self, turn, event))
    }
}

/// ISO 8601 UTC timestamp.
fn now_utc_iso8601() -> String {
    use std::time::SystemTime;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    // Format as ISO 8601 with milliseconds.
    let secs = now.as_secs();
    let millis = now.subsec_millis();
    // Convert to date-time components.
    let (year, month, day, hour, minute, second) = unix_to_utc(secs);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        year, month, day, hour, minute, second, millis
    )
}

/// Convert Unix epoch seconds to (year, month, day, hour, minute, second).
///
/// Minimal implementation — avoids pulling in chrono/time crate.
fn unix_to_utc(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    let second = secs % 60;
    let total_minutes = secs / 60;
    let minute = total_minutes % 60;
    let total_hours = total_minutes / 60;
    let hour = total_hours % 24;
    let mut days = total_hours / 24;

    // Calculate year from days since epoch (1970-01-01).
    let mut year = 1970u64;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }

    // Calculate month from remaining days.
    let month_days: [u64; 12] = if is_leap(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut month = 1u64;
    for &md in &month_days {
        if days < md {
            break;
        }
        days -= md;
        month += 1;
    }
    let day = days + 1;

    (year, month, day, hour, minute, second)
}

fn is_leap(year: u64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::audit::AuditEvent;
    use tempfile::TempDir;

    #[tokio::test]
    async fn creates_audit_directory_on_open() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        assert!(!base.join("audit").exists());

        let _log = AuditLog::open(base, "test-session").await.unwrap();
        assert!(base.join("audit").exists());
    }

    #[tokio::test]
    async fn writes_valid_jsonl_with_envelope() {
        let tmp = TempDir::new().unwrap();
        let log = AuditLog::open(tmp.path(), "cli:my-feature").await.unwrap();

        log.emit(
            7,
            AuditEvent::ToolCall {
                tool: "bash".into(),
                call_id: "call_abc".into(),
                arguments: r#"{"command":"test"}"#.into(),
            },
        )
        .await
        .unwrap();

        let path = AuditLog::file_path(tmp.path(), "cli:my-feature");
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 1);

        let val: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(val["session"], "cli:my-feature");
        assert_eq!(val["turn"], 7);
        assert_eq!(val["event"], "tool_call");
        assert_eq!(val["tool"], "bash");
        assert_eq!(val["call_id"], "call_abc");
        // ts should be ISO 8601
        let ts = val["ts"].as_str().unwrap();
        assert!(ts.ends_with('Z'));
        assert!(ts.contains('T'));
    }

    #[tokio::test]
    async fn appends_multiple_events_in_order() {
        let tmp = TempDir::new().unwrap();
        let log = AuditLog::open(tmp.path(), "multi").await.unwrap();

        log.emit(
            1,
            AuditEvent::ToolCall {
                tool: "bash".into(),
                call_id: "c1".into(),
                arguments: "{}".into(),
            },
        )
        .await
        .unwrap();
        log.emit(
            1,
            AuditEvent::ToolResult {
                call_id: "c1".into(),
                tool: "bash".into(),
                is_error: false,
                content_tokens: 100,
                content_preview: "ok".into(),
            },
        )
        .await
        .unwrap();
        log.emit(
            2,
            AuditEvent::LlmTurnStart {
                input_tokens_estimate: 5000,
                message_count: 10,
            },
        )
        .await
        .unwrap();
        log.emit(
            2,
            AuditEvent::LlmTurnEnd {
                input_tokens: 5000,
                output_tokens: 500,
                stop_reason: "end_turn".into(),
                duration_ms: 2000,
            },
        )
        .await
        .unwrap();

        let path = AuditLog::file_path(tmp.path(), "multi");
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 4);

        let v0: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        let v1: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        let v2: serde_json::Value = serde_json::from_str(lines[2]).unwrap();
        let v3: serde_json::Value = serde_json::from_str(lines[3]).unwrap();
        assert_eq!(v0["event"], "tool_call");
        assert_eq!(v1["event"], "tool_result");
        assert_eq!(v2["event"], "llm_turn_start");
        assert_eq!(v3["event"], "llm_turn_end");
    }

    #[tokio::test]
    async fn flushes_on_every_write() {
        let tmp = TempDir::new().unwrap();
        let log = AuditLog::open(tmp.path(), "flush-test").await.unwrap();

        log.emit(
            1,
            AuditEvent::ToolCall {
                tool: "bash".into(),
                call_id: "c1".into(),
                arguments: "{}".into(),
            },
        )
        .await
        .unwrap();

        // Read without closing the log — file should be readable
        let path = AuditLog::file_path(tmp.path(), "flush-test");
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(content.lines().count(), 1);

        // Emit another event
        log.emit(
            2,
            AuditEvent::ToolCall {
                tool: "read".into(),
                call_id: "c2".into(),
                arguments: "{}".into(),
            },
        )
        .await
        .unwrap();

        let content2 = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(content2.lines().count(), 2);
    }

    #[tokio::test]
    async fn sanitizes_session_key_for_filename() {
        let tmp = TempDir::new().unwrap();
        let _log = AuditLog::open(tmp.path(), "cli:my-feature").await.unwrap();

        let path = AuditLog::file_path(tmp.path(), "cli:my-feature");
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), "cli_my-feature.jsonl");
    }

    #[tokio::test]
    async fn all_event_types_write_successfully() {
        let tmp = TempDir::new().unwrap();
        let log = AuditLog::open(tmp.path(), "all-types").await.unwrap();

        let events = vec![
            AuditEvent::ToolCall {
                tool: "bash".into(),
                call_id: "c1".into(),
                arguments: "{}".into(),
            },
            AuditEvent::ToolResult {
                call_id: "c1".into(),
                tool: "bash".into(),
                is_error: false,
                content_tokens: 10,
                content_preview: "ok".into(),
            },
            AuditEvent::LlmTurnStart {
                input_tokens_estimate: 1000,
                message_count: 5,
            },
            AuditEvent::LlmTurnEnd {
                input_tokens: 1000,
                output_tokens: 200,
                stop_reason: "end_turn".into(),
                duration_ms: 500,
            },
            AuditEvent::WorkflowStep {
                action: "check".into(),
                step_index: 1,
                step_key: "tests".into(),
                step_label: "Write tests".into(),
                template_id: "feature".into(),
            },
            AuditEvent::WorkflowTransition {
                from_mode: "selector".into(),
                to_mode: "active".into(),
                template_id: Some("feature".into()),
                issue: None,
            },
            AuditEvent::ContextPruned {
                messages_dropped: 5,
                tool_results_collapsed: 0,
                tokens_before: 100_000,
                tokens_after: 80_000,
            },
            AuditEvent::SubagentSpawned {
                agent_id: "reviewer".into(),
                task_preview: "Review code".into(),
                system_preview: "You are a reviewer".into(),
            },
            AuditEvent::SubagentCmd {
                agent_id: "reviewer".into(),
                command: "status".into(),
            },
            AuditEvent::GuardBlocked {
                command_preview: "git push".into(),
                guard_message: "Not ready".into(),
                before_step_key: "commit".into(),
            },
            AuditEvent::Error {
                source: "provider".into(),
                tool: None,
                message: "rate limited".into(),
            },
        ];

        for (i, event) in events.into_iter().enumerate() {
            log.emit(i as u32 + 1, event).await.unwrap();
        }

        let path = AuditLog::file_path(tmp.path(), "all-types");
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 11);

        // Verify each line is valid JSON with envelope
        for (i, line) in lines.iter().enumerate() {
            let val: serde_json::Value = serde_json::from_str(line).unwrap();
            assert!(val["ts"].is_string(), "line {} missing ts", i);
            assert_eq!(val["session"], "all-types");
            assert_eq!(val["turn"], i as u64 + 1);
            assert!(val["event"].is_string(), "line {} missing event", i);
        }
    }

    #[test]
    fn unix_to_utc_epoch() {
        assert_eq!(unix_to_utc(0), (1970, 1, 1, 0, 0, 0));
    }

    #[test]
    fn unix_to_utc_known_date() {
        // 2026-03-28T00:00:00Z = 1774656000
        let (y, m, d, h, mi, s) = unix_to_utc(1_774_656_000);
        assert_eq!((y, m, d, h, mi, s), (2026, 3, 28, 0, 0, 0));
    }

    #[test]
    fn now_utc_iso8601_format() {
        let ts = now_utc_iso8601();
        assert!(ts.ends_with('Z'));
        assert!(ts.contains('T'));
        assert_eq!(ts.len(), 24); // "YYYY-MM-DDTHH:MM:SS.mmmZ"
    }
}
