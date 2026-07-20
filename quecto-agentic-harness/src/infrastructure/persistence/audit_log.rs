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
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), DomainError>> + Send + '_>>
    {
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
#[path = "audit_log_cov_tests.rs"]
mod cov_tests;

#[cfg(test)]
#[path = "audit_log_tests.rs"]
mod tests;
