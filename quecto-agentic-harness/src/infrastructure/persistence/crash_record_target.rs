//! The armed crash target (#2192): where this process's panics are
//! recorded, the session it records for, and the event log's crash line —
//! each swapped atomically on a session switch, so the hook takes no lock.
use std::path::PathBuf;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::application::audit::ports::AuditSink;
use crate::domain::audit::AuditEvent;
use crate::domain::crash_record::CrashRecord;
use crate::domain::session_identity::SessionIdentity;

use super::super::audit_log::{AuditCrashLine, AuditLog};
use super::{RecordDir, SessionRecords};

/// Where this process's panics are recorded. Set once; the hook reads it
/// without a lock and swaps nothing but the session and crash-line
/// pointers.
#[derive(Debug)]
pub struct Armed {
    base_dir: PathBuf,
    dir: OnceLock<RecordDir>,
    /// The event log's crash line; null for none. Each value is leaked, as
    /// a session's records are, so a hook reading the old one is never left
    /// dangling (#2192 review: it follows a session switch).
    event_log: AtomicPtr<AuditCrashLine>,
    /// The session recorded for; null while disarmed. Each value is a
    /// leaked `SessionRecords`: a switch leaks one small allocation, never
    /// freed, so a hook reading the old one is never left dangling.
    session: AtomicPtr<SessionRecords>,
}

impl Armed {
    /// A target under `base_dir`, recording for `session_key` (none for a
    /// session that leaves nothing behind), with the log's crash line.
    pub fn new(
        base_dir: impl Into<PathBuf>,
        session_key: Option<&str>,
        event_log: Option<AuditCrashLine>,
    ) -> Self {
        AuditLog::warm_host_name();
        let armed = Self {
            base_dir: base_dir.into(),
            dir: OnceLock::new(),
            event_log: AtomicPtr::new(leaked(event_log)),
            session: AtomicPtr::new(std::ptr::null_mut()),
        };
        armed.follow_records(session_key);
        armed.sweep();
        armed
    }

    /// Sweep the crash directory of records grown stale (#2192 review):
    /// once, as the target is made at startup. A directory this process
    /// did not create (no session of its own) is swept if it is there.
    fn sweep(&self) {
        match self.dir.get() {
            Some(dir) => super::sweep::sweep(dir),
            None => {
                if let Ok(dir) = RecordDir::existing(&self.base_dir) {
                    super::sweep::sweep(&dir);
                }
            }
        }
    }

    /// Record for `session_key` from now on (none: record nothing), after
    /// clearing what an earlier run of it left. When the event log was the
    /// departing session's own, the log of `session_key` is opened (same
    /// parent, same cap) and its crash line taken instead (#2192 review):
    /// that log is answered, for the agent to write its events to — the
    /// fatal `error` event then lands in the log of the session it
    /// happened in. Any other log (one of its own key) stays as it was.
    pub fn follow(&self, session_key: Option<&str>) -> Option<AuditLog> {
        let departing = self.current().map(|(records, _)| records.key().to_string());
        self.follow_records(session_key);
        let arriving = self.current().map(|(records, _)| records.key().to_string());
        let line = self.event_log()?;
        let (departing, arriving) = match (departing, arriving) {
            (Some(departing), Some(arriving))
                if departing == line.session_key() && arriving != departing =>
            {
                (departing, arriving)
            }
            _ => return None,
        };
        match line.log_for(&self.base_dir, &arriving) {
            Ok(log) => {
                self.event_log
                    .store(leaked(log.crash_line()), Ordering::Release);
                Some(log)
            }
            Err(error) => {
                tracing::warn!(%error, departing, arriving, "the event log could not follow the session");
                None
            }
        }
    }

    /// The event log's crash line, if there is one. Hook-safe.
    fn event_log(&self) -> Option<&AuditCrashLine> {
        let line = self.event_log.load(Ordering::Acquire);
        // Non-null pointers come from `leaked`, never freed.
        // SAFETY: such a pointer stays valid for the life of the process.
        unsafe { line.as_ref() }
    }

    fn follow_records(&self, session_key: Option<&str>) {
        // Only a persisted session leaves records: the domain's own
        // classification of a key.
        let next = session_key
            .map(SessionIdentity::from_persisted_key)
            .and_then(|identity| identity.persisted_key().map(SessionRecords::new))
            .filter(|records| match self.directory() {
                Some(dir) => {
                    records.clear(dir);
                    true
                }
                None => false,
            })
            .map_or(std::ptr::null_mut(), |records| {
                Box::into_raw(Box::new(records))
            });
        self.session.store(next, Ordering::Release);
    }

    fn directory(&self) -> Option<&RecordDir> {
        match self.dir.get() {
            Some(dir) => Some(dir),
            None => match RecordDir::create(&self.base_dir) {
                Ok(dir) => Some(self.dir.get_or_init(|| dir)),
                Err(error) => {
                    tracing::warn!(%error, "no crash record can be kept");
                    None
                }
            },
        }
    }

    /// The session recorded for and its directory, if armed for one.
    fn current(&self) -> Option<(&SessionRecords, &RecordDir)> {
        let session = self.session.load(Ordering::Acquire);
        // Non-null pointers come from `Box::into_raw` in `follow`, never freed.
        // SAFETY: such a pointer stays valid for the life of the process.
        let session = unsafe { session.as_ref() }?;
        Some((session, self.dir.get()?))
    }

    /// Record a fatal panic: the fatal record and the log's last line.
    pub fn record_fatal(&self, record: &CrashRecord, source: &str, turn: u32) {
        if let Some((session, dir)) = self.current() {
            let _ = session.write_fatal(dir, record);
        }
        if let Some(log) = self.event_log() {
            let event = AuditEvent::Error {
                source: source.to_string(),
                // Only the call whose own code panicked: a panic outside any
                // call is attributed to none of those running.
                tool: record.call.clone(),
                message: record.panic.message.clone(),
                location: record.panic.location.clone(),
            };
            let _ = log.write(turn, event);
        }
    }

    /// Record the panic the call scoped `scope` is containing, for the case
    /// the process ends before the call does. Answers the session it was
    /// written under, to withdraw it from there: the process may follow
    /// another session before the call ends.
    pub fn record_provisional(&self, scope: u64, record: &CrashRecord) -> Option<String> {
        let (session, dir) = self.current()?;
        match session.write_provisional(dir, scope, record) {
            Ok(()) => Some(session.key().to_string()),
            Err(_) => None,
        }
    }

    /// The call scoped `scope` ended: withdraw its provisional record from
    /// the session it was written under (`written_under`), and from the
    /// current one, where an unnoted record would be. A missing record is
    /// already withdrawn.
    pub fn withdraw(&self, scope: u64, written_under: Option<&str>) {
        let Some(dir) = self.dir.get() else {
            return;
        };
        let noted = written_under
            .map(SessionIdentity::from_persisted_key)
            .and_then(|identity| identity.persisted_key().map(SessionRecords::new));
        if let Some(records) = &noted {
            let _ = records.withdraw(dir, scope);
        }
        match (self.current(), written_under) {
            // Withdrawn above: it is the session it was noted under.
            (Some((current, _)), Some(noted)) if current.key() == noted => {}
            (Some((current, _)), _) => {
                let _ = current.withdraw(dir, scope);
            }
            (None, _) => {}
        }
    }
}

/// What the agent prepared at startup, armed once its session is claimed.
#[derive(Debug, Default)]
pub struct CrashTarget {
    pub base_dir: Option<PathBuf>,
    /// `None` for a session that leaves nothing behind.
    pub session_key: Option<String>,
    pub event_log: Option<AuditCrashLine>,
}

static PREPARED: Mutex<Option<CrashTarget>> = Mutex::new(None);
static ARMED: OnceLock<Armed> = OnceLock::new();

/// Prepare `target` for this process; it takes effect when [`arm_prepared`]
/// runs, once the session is claimed.
pub fn prepare(target: CrashTarget) {
    *PREPARED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(target);
}

/// Arm the prepared target: this process owns its session now. Records an
/// earlier run of the session left are removed first: they described that
/// run, and a parent reading them now would blame this one. The first arm
/// in a process holds; a later one only follows its session.
pub fn arm_prepared() {
    let prepared = PREPARED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take();
    let Some(CrashTarget {
        base_dir: Some(base_dir),
        session_key,
        event_log,
    }) = prepared
    else {
        return;
    };
    let mut fresh = Some((base_dir, event_log));
    let armed = ARMED.get_or_init(|| {
        let (base_dir, event_log) = fresh.take().expect("initialised once");
        Armed::new(base_dir, session_key.as_deref(), event_log)
    });
    if fresh.is_some() {
        armed.follow_records(session_key.as_deref());
    }
}

/// [`arm_prepared`], once `claimed` shows the session was claimed.
pub fn claimed<T>(claimed: T) -> T {
    arm_prepared();
    claimed
}

/// The session switched to `session_key` (none: one that leaves nothing
/// behind), claimed by this process: record for it from now on. Answers
/// the event log the agent writes to from now on, when it followed too.
pub fn follow(session_key: Option<&str>) -> Option<Arc<dyn AuditSink>> {
    let log = ARMED.get()?.follow(session_key)?;
    Some(Arc::new(log))
}

/// Record a fatal panic in the armed target, if there is one. Hook-safe.
pub fn record_fatal(record: &CrashRecord, source: &str, turn: u32) {
    if let Some(armed) = ARMED.get() {
        armed.record_fatal(record, source, turn);
    }
}

/// Record the panic the call scoped `scope` is containing, answering the
/// session it was written under. Hook-safe.
pub fn record_provisional(scope: u64, record: &CrashRecord) -> Option<String> {
    ARMED.get()?.record_provisional(scope, record)
}

/// The call scoped `scope` ended, or its panic was caught: withdraw its
/// provisional record, written under `written_under` if that was noted.
pub fn withdraw(scope: u64, written_under: Option<&str>) {
    if let Some(armed) = ARMED.get() {
        armed.withdraw(scope, written_under);
    }
}

/// `value` leaked, as a pointer the hook may read for the life of the
/// process; null for none.
fn leaked<T>(value: Option<T>) -> *mut T {
    value.map_or(std::ptr::null_mut(), |value| Box::into_raw(Box::new(value)))
}
