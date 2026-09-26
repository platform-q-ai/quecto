//! Append-only audit log writer.
//!
//! Writes one JSON line per event to `<base_dir>/audit/<session_key>.jsonl`.
//! Flushed on every write to survive crashes.

use std::path::{Path, PathBuf};

use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use crate::application::audit::ports::AuditSink;
use crate::domain::audit::{AuditEnvelope, AuditEvent};
use crate::domain::error::DomainError;
use crate::infrastructure::persistence::filename::sanitize_session_key;

/// The most one session's log holds before it stops (#2150): a runaway
/// session cannot fill the disk.
pub const DEFAULT_CAP_BYTES: u64 = 256 * 1024 * 1024;

/// Append-only audit log handle for a single session.
///
/// Uses a raw `tokio::fs::File` (no `BufWriter`) because every `emit()` call
/// flushes immediately for crash durability — buffering would be negated.
pub struct AuditLog {
    writer: Mutex<Writer>,
    session_key: String,
    parent: Option<String>,
    cap_bytes: u64,
}

/// The open file and how much it holds.
struct Writer {
    file: tokio::fs::File,
    written: u64,
    capped: bool,
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
        let base_dir = base_dir.to_path_buf();
        let key = session_key.to_string();
        tokio::task::spawn_blocking(move || Self::open_sync(&base_dir, &key))
            .await
            .map_err(|e| DomainError::Session(format!("failed to open audit log: {e}")))?
    }

    /// Open (or create) the audit log file for the given session (sync).
    ///
    /// The directory is private (0700) and the file too (0600), tightened
    /// when an earlier version left them looser (#2150).
    pub fn open_sync(base_dir: &Path, session_key: &str) -> Result<Self, DomainError> {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
        let audit_dir = base_dir.join("audit");
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&audit_dir)
            .map_err(|e| DomainError::Session(format!("failed to create audit dir: {e}")))?;
        std::fs::set_permissions(&audit_dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| DomainError::Session(format!("failed to secure audit dir: {e}")))?;

        let path = Self::file_path(base_dir, session_key);
        let std_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| DomainError::Session(format!("failed to open audit log: {e}")))?;
        std_file
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| DomainError::Session(format!("failed to secure audit log: {e}")))?;
        let written = std_file
            .metadata()
            .map_err(|e| DomainError::Session(format!("failed to read audit log: {e}")))?
            .len();

        Ok(Self {
            writer: Mutex::new(Writer {
                file: tokio::fs::File::from_std(std_file),
                written,
                capped: false,
            }),
            session_key: session_key.to_string(),
            parent: None,
            cap_bytes: DEFAULT_CAP_BYTES,
        })
    }

    /// Name this session's parent in every record: it is a sub-agent.
    pub fn with_parent(mut self, parent: Option<String>) -> Self {
        self.parent = parent;
        self
    }

    /// Stop at `cap_bytes` instead of [`DEFAULT_CAP_BYTES`].
    pub fn with_cap(mut self, cap_bytes: u64) -> Self {
        assert!(cap_bytes > 0, "an audit log holds something");
        self.cap_bytes = cap_bytes;
        self
    }

    fn line(&self, turn: u32, event: AuditEvent) -> Result<String, DomainError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let envelope = AuditEnvelope {
            ts: now_utc_iso8601(),
            unix_ms: u64::try_from(now.as_millis()).unwrap_or(u64::MAX),
            pid: std::process::id(),
            session: self.session_key.clone(),
            parent: self.parent.clone(),
            turn,
            event,
        };
        let mut line =
            serde_json::to_string(&envelope).map_err(|e| DomainError::Other(e.to_string()))?;
        line.push('\n');
        Ok(line)
    }

    /// Emit a single audit event.
    ///
    /// Serialises with envelope fields, writes one JSONL line, and flushes.
    /// The flush is critical — the log must survive crashes. A line that
    /// would take the file past its cap is not written: one `log_capped`
    /// record is, and nothing after it.
    pub async fn emit(&self, turn: u32, event: AuditEvent) -> Result<(), DomainError> {
        let line = self.line(turn, event)?;
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
        let fits = writer.written.saturating_add(line.len() as u64) <= self.cap_bytes;
        let line = match (writer.capped, fits) {
            (false, true) => line,
            (false, false) => {
                writer.capped = true;
                self.line(
                    turn,
                    AuditEvent::LogCapped {
                        cap_bytes: self.cap_bytes,
                    },
                )?
            }
            (true, _) => return Ok(()),
        };
        writer
            .file
            .write_all(line.as_bytes())
            .await
            .map_err(|e| DomainError::Session(format!("audit log write failed: {e}")))?;
        writer
            .file
            .flush()
            .await
            .map_err(|e| DomainError::Session(format!("audit log flush failed: {e}")))?;
        writer.written = writer.written.saturating_add(line.len() as u64);

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
