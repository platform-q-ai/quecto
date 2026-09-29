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
use crate::infrastructure::persistence::filename::{digest_session_key, sanitize_session_key};

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
    /// A second handle on the same append-mode file, for the one line a
    /// dying process writes from its panic hook (#2192).
    crash_file: Option<std::sync::Arc<std::fs::File>>,
    /// Set once the log is capped — before its `log_capped` record is
    /// written — and shared with the crash line, which writes nothing once
    /// it is set (#2192 review: nothing follows `log_capped`).
    stopped: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// The bytes of the file taken so far — what it held when opened, and
    /// every line reserved since — shared with the crash line (#2192
    /// review): each writer reserves its line's bytes here, atomically and
    /// before it writes, so the two together never pass the cap. No lock:
    /// the panic hook reserves too.
    reserved: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

/// Where a panic hook writes the `error` event of a fatal panic (#2192),
/// and a board op its `swarm_op` (#2303, [`AuditCrashLine::append`]): the
/// session's log, through its own append-mode handle, synchronously.
/// One `write` of one line: on an `O_APPEND` file it lands whole after any
/// line already written. The async writer also writes each line with one
/// `write`, but a regular file does not promise that a concurrent write is
/// not interleaved with one in progress, whatever its size (a line with a
/// long message and location is several KiB), so a line being written at
/// that instant may be split. The line is written only while the log is
/// not yet capped and its bytes can be reserved within the cap, from the
/// budget the async writer reserves from too: the cap is exact.
#[derive(Debug, Clone)]
pub struct AuditCrashLine {
    file: std::sync::Arc<std::fs::File>,
    session_key: String,
    parent: Option<String>,
    cap_bytes: u64,
    stopped: std::sync::Arc<std::sync::atomic::AtomicBool>,
    reserved: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl AuditCrashLine {
    /// Append `event`, filed under `turn`, as one line, when the log is not
    /// capped and its bytes can be reserved under the cap. Never panics and
    /// takes no lock; a failure (or a full or capped log) is an error — the
    /// crash record still says why.
    pub fn write(&self, turn: u32, event: AuditEvent) -> std::io::Result<()> {
        self.append(Some(turn), event)
    }

    /// [`Self::write`] for a record filed under no turn (`turn` `None`,
    /// written `null`): a board op's `swarm_op` (#2303), appended from the
    /// board call's own thread, synchronously, with no runtime and no lock.
    /// It shares the log's cap budget as a crash line does, and a line
    /// that does not fit is refused, not written.
    pub fn append(&self, turn: Option<u32>, event: AuditEvent) -> std::io::Result<()> {
        use std::io::Write;
        use std::sync::atomic::Ordering;
        let line = envelope_line(&self.session_key, self.parent.as_ref(), turn, event)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        let admitted = match self.stopped.load(Ordering::Acquire) {
            false => reserve(&self.reserved, line.len() as u64, self.cap_bytes),
            true => false,
        };
        match admitted {
            true => (&*self.file).write_all(line.as_bytes()),
            false => Err(std::io::Error::new(
                std::io::ErrorKind::StorageFull,
                "the event log is at its cap",
            )),
        }
    }

    /// The session this line's log is filed under.
    pub fn session_key(&self) -> &str {
        &self.session_key
    }

    /// The log of session `session_key` under `base_dir`, named for the
    /// same parent and held to the same cap as this line's (#2192 review:
    /// the log follows a session switch, and its crash line with it).
    pub fn log_for(&self, base_dir: &Path, session_key: &str) -> Result<AuditLog, DomainError> {
        Ok(AuditLog::open_sync(base_dir, session_key)?
            .with_parent(self.parent.clone())
            .with_cap(self.cap_bytes))
    }
}

/// The open file, and whether it is capped.
struct Writer {
    file: tokio::fs::File,
    capped: bool,
}

/// Reserve `len` bytes of the budget `reserved` holds under `cap`: taken
/// whole, atomically, or (it would pass the cap) not at all.
fn reserve(reserved: &std::sync::atomic::AtomicU64, len: u64, cap: u64) -> bool {
    use std::sync::atomic::Ordering;
    reserved
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |held| {
            held.checked_add(len).filter(|taken| *taken <= cap)
        })
        .is_ok()
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
    /// `~/.quecto` is shared with containers (#2150), so nothing here
    /// follows a link an agent could plant: the directory is opened without
    /// following one, and the file through that directory, likewise. Both
    /// are made private (0700, 0600) through their handles, tightening what
    /// an earlier version left looser; a tightening refused (someone else's
    /// file) leaves the log open, with a warning.
    pub fn open_sync(base_dir: &Path, session_key: &str) -> Result<Self, DomainError> {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        let audit_dir = base_dir.join("audit");
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&audit_dir)
            .map_err(|e| DomainError::Session(format!("failed to create audit dir: {e}")))?;
        let dir = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(&audit_dir)
            .map_err(|e| {
                DomainError::Session(format!(
                    "failed to open audit dir {} (a link is never followed): {e}",
                    audit_dir.display()
                ))
            })?;
        tighten(&dir, 0o700, &audit_dir);

        let filename = format!("{}.jsonl", sanitize_session_key(session_key));
        let shown = audit_dir.join(&filename);
        let std_file = open_within(&dir, &filename).map_err(|e| {
            DomainError::Session(format!(
                "failed to open audit log {} (only a regular file, never through a link): {e}",
                shown.display()
            ))
        })?;
        tighten(&std_file, 0o600, &shown);
        let crash_file = std_file.try_clone().ok().map(std::sync::Arc::new);
        let written = std_file
            .metadata()
            .map_err(|e| DomainError::Session(format!("failed to read audit log: {e}")))?
            .len();
        let stopped = ends_capped(&std_file, written);
        let capped = stopped || written >= DEFAULT_CAP_BYTES;

        Ok(Self {
            writer: Mutex::new(Writer {
                file: tokio::fs::File::from_std(std_file),
                capped,
            }),
            session_key: session_key.to_string(),
            parent: None,
            cap_bytes: DEFAULT_CAP_BYTES,
            crash_file,
            stopped: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(capped)),
            reserved: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(written)),
        })
    }

    /// Name this session's parent in every record: it is a sub-agent.
    pub fn with_parent(mut self, parent: Option<String>) -> Self {
        self.parent = parent;
        self
    }

    /// Stop at `cap_bytes` instead of [`DEFAULT_CAP_BYTES`]; a log already
    /// that full stays stopped.
    pub fn with_cap(mut self, cap_bytes: u64) -> Self {
        assert!(cap_bytes > 0, "an audit log holds something");
        self.cap_bytes = cap_bytes;
        let held = self.reserved.load(std::sync::atomic::Ordering::Acquire);
        let writer = self.writer.get_mut();
        writer.capped = writer.capped || held >= cap_bytes;
        self.stopped
            .store(writer.capped, std::sync::atomic::Ordering::Release);
        self
    }

    fn line(&self, turn: u32, event: AuditEvent) -> Result<String, DomainError> {
        envelope_line(&self.session_key, self.parent.as_ref(), Some(turn), event)
    }

    /// The handle a panic hook writes this log's last line through (#2192);
    /// `None` when the file could not be opened a second time.
    pub fn crash_line(&self) -> Option<AuditCrashLine> {
        self.crash_file.as_ref().map(|file| AuditCrashLine {
            file: file.clone(),
            session_key: self.session_key.clone(),
            parent: self.parent.clone(),
            cap_bytes: self.cap_bytes,
            stopped: self.stopped.clone(),
            reserved: self.reserved.clone(),
        })
    }

    /// Emit a single audit event.
    ///
    /// Serialises with envelope fields, writes one JSONL line, and flushes.
    /// The flush is critical — the log must survive crashes. A line that
    /// would take the file past its cap is not written: one `log_capped`
    /// record is, and nothing after it. A line's bytes are reserved from
    /// the budget the crash line shares before it is written, and stay
    /// reserved if the write fails (part of it may have landed).
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
        let fits = match writer.capped {
            false => reserve(&self.reserved, line.len() as u64, self.cap_bytes),
            true => return Ok(()),
        };
        let line = match fits {
            true => line,
            false => {
                writer.capped = true;
                // Before the record: a crash line after it would follow it.
                self.stopped
                    .store(true, std::sync::atomic::Ordering::Release);
                let capped = self.line(
                    turn,
                    AuditEvent::LogCapped {
                        cap_bytes: self.cap_bytes,
                    },
                )?;
                // The one record past the cap, counted all the same.
                self.reserved
                    .fetch_add(capped.len() as u64, std::sync::atomic::Ordering::AcqRel);
                capped
            }
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

        Ok(())
    }

    /// Return the path to the audit log file for a given session key.
    ///
    /// Useful for tests and external consumers.
    pub fn file_path(base_dir: &Path, session_key: &str) -> PathBuf {
        let filename = format!("{}.jsonl", sanitize_session_key(session_key));
        base_dir.join("audit").join(filename)
    }

    /// The directory a session's log and crash record live in (#2192).
    pub fn directory(base_dir: &Path) -> PathBuf {
        base_dir.join("audit")
    }

    /// The subdirectory of [`Self::directory`] crash records live in
    /// (#2192): their own, so the event logs of every session never crowd
    /// a record out of a bounded listing.
    pub const CRASH_DIRECTORY: &'static str = "crash";

    /// Where crash records live: [`Self::CRASH_DIRECTORY`] in the audit
    /// directory.
    pub fn crash_directory(base_dir: &Path) -> PathBuf {
        Self::directory(base_dir).join(Self::CRASH_DIRECTORY)
    }

    /// The name of a session's crash record in [`Self::crash_directory`],
    /// whether or not its log is kept (#2192): named by the key's digest
    /// ([`digest_session_key`]), never its readable form, so no two
    /// sessions share a record (#2192 review: `telegram:123` and
    /// `telegram_123` would).
    pub fn crash_record_name(session_key: &str) -> String {
        format!("{}.crash", digest_session_key(session_key))
    }

    /// Resolve the host name now, so a panic hook later reads it cached.
    pub fn warm_host_name() {
        let _ = host_name();
    }
}

/// One audit line: the envelope around `event`, newline-terminated.
fn envelope_line(
    session_key: &str,
    parent: Option<&String>,
    turn: Option<u32>,
    event: AuditEvent,
) -> Result<String, DomainError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let envelope = AuditEnvelope {
        ts: now_utc_iso8601(),
        unix_ms: u64::try_from(now.as_millis()).unwrap_or(u64::MAX),
        pid: std::process::id(),
        host: host_name(),
        session: session_key.to_string(),
        parent: parent.cloned(),
        turn,
        event,
    };
    let mut line =
        serde_json::to_string(&envelope).map_err(|e| DomainError::Other(e.to_string()))?;
    line.push('\n');
    Ok(line)
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

/// Open (or create, 0600) `name` in the directory `dir` names, for
/// appending: never through a link (`O_NOFOLLOW`), never blocking on a
/// planted FIFO (`O_NONBLOCK`), and only a regular file. `openat` needs no
/// `/proc`, which a container may not mount.
fn open_within(dir: &std::fs::File, name: &str) -> std::io::Result<std::fs::File> {
    use std::os::unix::io::{AsRawFd, FromRawFd};
    let name = std::ffi::CString::new(name)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "a NUL in the name"))?;
    let flags = libc::O_RDWR
        | libc::O_APPEND
        | libc::O_CREAT
        | libc::O_NOFOLLOW
        | libc::O_NONBLOCK
        | libc::O_CLOEXEC;
    // The returned descriptor is owned below.
    // SAFETY: `dir` is an open directory for the call and `name` is NUL-terminated.
    let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags, 0o600 as libc::c_uint) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `fd` was just opened and is owned by nothing else.
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    match file.metadata()?.file_type().is_file() {
        true => Ok(file),
        false => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "not a regular file",
        )),
    }
}

/// Whether the log already ends with its `log_capped` record: it stopped,
/// even though that record left it below the cap.
fn ends_capped(file: &std::fs::File, len: u64) -> bool {
    use std::os::unix::fs::FileExt;
    let tail = len.min(4096);
    let mut bytes = vec![0; tail as usize];
    if file.read_exact_at(&mut bytes, len - tail).is_err() {
        return false;
    }
    String::from_utf8_lossy(&bytes)
        .trim_end()
        .rsplit('\n')
        .next()
        .is_some_and(|last| last.contains(r#""event":"log_capped""#))
}

/// Make `file` private (`mode`) through its handle; best-effort.
fn tighten(file: &std::fs::File, mode: u32, shown: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Err(error) = file.set_permissions(std::fs::Permissions::from_mode(mode)) {
        tracing::warn!(%error, path = %shown.display(), "audit log left as it was: it could not be made private");
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

/// This host's name, read once (#2161). The bundled container scripts name
/// each container's host after the container (`quecto-<environment>`);
/// otherwise a container's host is its short ID, and on the host it is the
/// machine's name.
fn host_name() -> Option<String> {
    static HOST: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    HOST.get_or_init(|| {
        let mut buffer = [0u8; 256];
        // SAFETY: the buffer is writable for its whole length, which is passed.
        let status = unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) };
        host_from(status, &buffer)
    })
    .clone()
}

/// A host name from gethostname's status and buffer: none when the call
/// failed, the name was not terminated within the buffer, or it is empty.
fn host_from(status: libc::c_int, buffer: &[u8]) -> Option<String> {
    let end = buffer.iter().position(|byte| *byte == 0)?;
    (status == 0)
        .then(|| String::from_utf8_lossy(&buffer[..end]).into_owned())
        .filter(|name| !name.is_empty())
}

#[cfg(test)]
#[path = "audit_log_cov_tests.rs"]
mod cov_tests;

#[cfg(test)]
#[path = "audit_log_tests.rs"]
mod tests;
