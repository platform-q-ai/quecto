//! Append-only audit log writer.
//!
//! Writes one JSON line per event to `<base_dir>/audit/<session_key>.jsonl`.
//! Flushed on every write to survive crashes.

use std::path::{Path, PathBuf};

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::application::audit::ports::AuditSink;
use crate::domain::audit::{AuditEnvelope, AuditEvent};
use crate::domain::error::DomainError;
use crate::infrastructure::persistence::filename::{digest_session_key, sanitize_session_key};

/// The most one session's log holds before it stops (#2150): a runaway
/// session cannot fill the disk.
pub const DEFAULT_CAP_BYTES: u64 = 256 * 1024 * 1024;

/// Append-only audit log handle for a single session.
///
/// Every line is one `write` on an `O_APPEND` file, made under the log's
/// write gate ([`WriteGate`]): the async writer (from the blocking pool),
/// the board's `swarm_op` appender and the panic hook's crash line all
/// take it, so no two lines interleave however long they are, and no line
/// follows the `log_capped` record (#2303 round-2 review L1, L2). No
/// buffering: every line is on disk when its write returns.
pub struct AuditLog {
    file: Arc<std::fs::File>,
    gate: WriteGate,
    session_key: String,
    parent: Option<String>,
    cap_bytes: u64,
    /// A second handle on the same append-mode file, for the one line a
    /// dying process writes from its panic hook (#2192).
    crash_file: Option<Arc<std::fs::File>>,
}

/// What every writer of one log shares (#2192, #2303): the gate each line
/// is written under, whether the log is capped, and the bytes of the file
/// taken so far.
#[derive(Debug, Clone, Default)]
struct WriteGate {
    /// Held around each line's check, reservation and write.
    held: Arc<Mutex<()>>,
    /// Set once the log is capped, before its `log_capped` record is
    /// written: nothing is written once it is set.
    stopped: Arc<AtomicBool>,
    /// The bytes of the file taken so far — what it held when opened, and
    /// every line written since, the `log_capped` record included.
    reserved: Arc<AtomicU64>,
}

/// How long the panic hook waits for the write gate before giving up on
/// its line: a writer holds it only for one `write`, and a hook must never
/// hang a dying process.
const CRASH_GATE_WAIT: Duration = Duration::from_secs(1);

/// What became of a line offered to the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Appended {
    /// Written.
    Written,
    /// It did not fit: the log's one `log_capped` record was written in its
    /// place, and the log is stopped.
    Capped,
    /// Not written: the log was already stopped, or the line did not fit
    /// and the writer does not cap the log.
    Refused,
}

impl WriteGate {
    fn new(stopped: bool, written: u64) -> Self {
        Self {
            held: Arc::default(),
            stopped: Arc::new(AtomicBool::new(stopped)),
            reserved: Arc::new(AtomicU64::new(written)),
        }
    }

    /// The gate, waited for without bound, or for at most `bound`.
    fn hold(&self, bound: Option<Duration>) -> std::io::Result<MutexGuard<'_, ()>> {
        let Some(bound) = bound else {
            return Ok(self.held.lock().unwrap_or_else(PoisonError::into_inner));
        };
        let started = Instant::now();
        loop {
            match self.held.try_lock() {
                Ok(held) => return Ok(held),
                Err(std::sync::TryLockError::Poisoned(held)) => return Ok(held.into_inner()),
                Err(std::sync::TryLockError::WouldBlock) if started.elapsed() < bound => {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(std::sync::TryLockError::WouldBlock) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::WouldBlock,
                        "the event log stayed busy",
                    ));
                }
            }
        }
    }

    /// Appends `line` to `file` under the gate, within `cap` bytes: nothing
    /// once the log is stopped; the line when its bytes fit; otherwise, for
    /// a writer that caps the log (`capped` gives its `log_capped` record),
    /// that record in the line's place — the one record past the cap,
    /// counted all the same — and the log stops, the flag set before the
    /// record is written. A line's bytes stay reserved if its write fails
    /// (part of it may have landed).
    fn append(
        &self,
        file: &std::fs::File,
        cap: u64,
        line: &str,
        capped: Option<&dyn Fn() -> Option<String>>,
        bound: Option<Duration>,
    ) -> std::io::Result<Appended> {
        use std::io::Write;
        let _held = self.hold(bound)?;
        if self.stopped.load(Ordering::Acquire) {
            return Ok(Appended::Refused);
        }
        if reserve(&self.reserved, line.len() as u64, cap) {
            return (&*file)
                .write_all(line.as_bytes())
                .map(|()| Appended::Written);
        }
        let Some(record) = capped.and_then(|capped| capped()) else {
            return Ok(Appended::Refused);
        };
        self.stopped.store(true, Ordering::Release);
        self.reserved
            .fetch_add(record.len() as u64, Ordering::AcqRel);
        (&*file)
            .write_all(record.as_bytes())
            .map(|()| Appended::Capped)
    }
}

/// Where a panic hook writes the `error` event of a fatal panic (#2192),
/// and a board op its `swarm_op` (#2303, [`AuditCrashLine::append`]): the
/// session's log, through its own append-mode handle, synchronously, one
/// `write` of one line under the log's write gate, so it never interleaves
/// with another writer's line. The line is written only while the log is
/// not yet capped and its bytes can be reserved within the cap, from the
/// budget every writer reserves from: the cap is exact.
#[derive(Debug, Clone)]
pub struct AuditCrashLine {
    file: Arc<std::fs::File>,
    session_key: String,
    parent: Option<String>,
    cap_bytes: u64,
    gate: WriteGate,
}

impl AuditCrashLine {
    /// Append `event`, filed under `turn`, as one line, when the log is not
    /// capped and its bytes can be reserved under the cap. Never panics,
    /// and waits for the write gate at most [`CRASH_GATE_WAIT`]: a panic
    /// hook never hangs. A failure (or a full or capped log) is an error —
    /// the crash record still says why. A line that does not fit leaves the
    /// log as it was: a dying process does not cap it.
    pub fn write(&self, turn: u32, event: AuditEvent) -> std::io::Result<()> {
        let line = self.line(Some(turn), event)?;
        let appended = self.gate.append(
            &self.file,
            self.cap_bytes,
            &line,
            None,
            Some(CRASH_GATE_WAIT),
        )?;
        written(appended)
    }

    /// Append `event` filed under no turn (`turn` `None`, written `null`):
    /// a board op's `swarm_op` (#2303), from the board call's own thread,
    /// synchronously, with no runtime. It shares the log's cap budget and
    /// write gate with every writer; a record that does not fit is refused,
    /// not written, and when it is the first line that does not fit, the
    /// log's one `log_capped` record is written in its place and the log
    /// stops, as the async writer stops it.
    pub fn append(&self, turn: Option<u32>, event: AuditEvent) -> std::io::Result<()> {
        let line = self.line(turn, event)?;
        let capped = || {
            let cap_bytes = self.cap_bytes;
            envelope_line(
                &self.session_key,
                self.parent.as_ref(),
                turn,
                AuditEvent::LogCapped { cap_bytes },
            )
            .ok()
        };
        let appended = self.gate.append(
            &self.file,
            self.cap_bytes,
            &line,
            Option::<&dyn Fn() -> Option<String>>::None
                .or(None)
                .filter(|_| {
                    let _ = &capped;
                    false
                }),
            None,
        )?;
        written(appended)
    }

    fn line(&self, turn: Option<u32>, event: AuditEvent) -> std::io::Result<String> {
        envelope_line(&self.session_key, self.parent.as_ref(), turn, event)
            .map_err(|e| std::io::Error::other(e.to_string()))
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

/// A line that was written is `Ok`; one that was not is `StorageFull`.
fn written(appended: Appended) -> std::io::Result<()> {
    match appended {
        Appended::Written => Ok(()),
        Appended::Capped | Appended::Refused => Err(std::io::Error::new(
            std::io::ErrorKind::StorageFull,
            "the event log is at its cap",
        )),
    }
}

/// Reserve `len` bytes of the budget `reserved` holds under `cap`: taken
/// whole, atomically, or (it would pass the cap) not at all.
fn reserve(reserved: &AtomicU64, len: u64, cap: u64) -> bool {
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
        let crash_file = std_file.try_clone().ok().map(Arc::new);
        let written = std_file
            .metadata()
            .map_err(|e| DomainError::Session(format!("failed to read audit log: {e}")))?
            .len();
        let capped = ends_capped(&std_file, written) || written >= DEFAULT_CAP_BYTES;

        Ok(Self {
            file: Arc::new(std_file),
            gate: WriteGate::new(capped, written),
            session_key: session_key.to_string(),
            parent: None,
            cap_bytes: DEFAULT_CAP_BYTES,
            crash_file,
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
        let held = self.gate.reserved.load(Ordering::Acquire);
        if held >= cap_bytes {
            self.gate.stopped.store(true, Ordering::Release);
        }
        self
    }

    /// The appender every writer of this log shares, over `file`.
    fn appender(&self, file: Arc<std::fs::File>) -> AuditCrashLine {
        AuditCrashLine {
            file,
            session_key: self.session_key.clone(),
            parent: self.parent.clone(),
            cap_bytes: self.cap_bytes,
            gate: self.gate.clone(),
        }
    }

    /// The handle a panic hook writes this log's last line through (#2192);
    /// `None` when the file could not be opened a second time.
    pub fn crash_line(&self) -> Option<AuditCrashLine> {
        self.crash_file
            .as_ref()
            .map(|file| self.appender(file.clone()))
    }

    /// Emit a single audit event.
    ///
    /// Serialises it with its envelope and writes one JSONL line, from the
    /// blocking pool, under the log's write gate: one `write` on the
    /// `O_APPEND` file, which no other writer's line can interleave with
    /// whatever its length (the gate, not the size of the line, keeps it
    /// whole). There is no buffer to flush: the line is on disk when the
    /// write returns, so a crash never loses it. A line that would take the
    /// file past its cap is not written: one `log_capped` record is, and
    /// nothing after it, whichever writer reached the cap first.
    pub async fn emit(&self, turn: u32, event: AuditEvent) -> Result<(), DomainError> {
        let appender = self.appender(self.file.clone());
        let line = appender
            .line(Some(turn), event)
            .map_err(|e| DomainError::Other(e.to_string()))?;
        let appended = tokio::task::spawn_blocking(move || {
            let capped = || {
                let cap_bytes = appender.cap_bytes;
                envelope_line(
                    &appender.session_key,
                    appender.parent.as_ref(),
                    Some(turn),
                    AuditEvent::LogCapped { cap_bytes },
                )
                .ok()
            };
            appender.gate.append(
                &appender.file,
                appender.cap_bytes,
                &line,
                Some(&capped),
                None,
            )
        })
        .await
        .map_err(|e| DomainError::Session(format!("audit log write failed: {e}")))?;
        appended
            .map(|_appended| ())
            .map_err(|e| DomainError::Session(format!("audit log write failed: {e}")))
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

#[cfg(test)]
#[path = "audit_log_gate_tests.rs"]
mod gate_tests;
