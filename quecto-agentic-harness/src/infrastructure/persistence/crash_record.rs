//! The crash record a dying harness leaves (#2192), and how it is read.
//!
//! At startup the agent prepares its target: the directory its crash
//! records go in (`<base>/audit/crash`, their own, beside the event logs so
//! no session's log crowds a record out of a listing) and, when it keeps an
//! event log, that log's crash line. The target is armed only once the
//! session is claimed, so a process refused the session never touches the
//! owner's records, and it follows the session: a switch (`/resume`,
//! `/new`) moves it to the new key once that is this process's, and a switch
//! to a session that leaves nothing behind disarms it.
//!
//! A session's records are files in the crash directory, named by the
//! digest of its key (`<digest>`:
//! [`digest_session_key`](super::filename::digest_session_key), so no two
//! sessions share a name), each saying the exact key it is for:
//! - `<digest>.crash`: a fatal panic, written by the hook just before the
//!   process ends. Only a fatal record is ever written there.
//! - `<digest>.crash.provisional.<pid>.<scope>`: a panic a tool call is
//!   containing, one file per call, written while it unwinds and removed —
//!   only that file, by name, from the session it was written under — when
//!   the call ends or catches the panic itself. One still there means the
//!   process ended before the call did.
//!
//! A record is taken only for the session it names; nothing else under its
//! name is. A new run of a session clears what an earlier one left, the
//! temporary files of writes cut short too, and each start sweeps records
//! older than [`STALE_RECORD_AGE`] of every session (`crash_record_sweep`).
//!
//! A reader asks for the records of one process (the pid it expects, when
//! it knows one): the fatal record only if it names that pid, else the
//! newest provisional record under that pid's name that names it too. A
//! record naming another pid, however new, does not stand in for it. The
//! pid is the record's own, unauthenticated word: anyone who can write the
//! directory (this user, in the same-uid trust model) can forge a record
//! naming the expected pid. The hook takes no lock: the armed
//! target is set once and the session it records for is swapped
//! atomically. Everything the hook calls is synchronous and never panics: a
//! record that cannot be written is simply missing. `~/.quecto` is shared
//! with containers, so nothing here follows a link: the directory is held
//! open (`O_DIRECTORY | O_NOFOLLOW`, each of `audit` and `crash` in turn)
//! and every file is reached — and the directory listed, a bounded number
//! of entries, a listing cut short saying so — through it; a record is
//! written whole (a temporary file with an unpredictable name, renamed into
//! place) and read only from a regular file of bounded size, without
//! blocking on a FIFO.
use std::ffi::CString;
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::domain::crash_record::CrashRecord;

use super::audit_log::AuditLog;

/// The largest crash record read back: a record is one short line.
pub const MAX_RECORD_BYTES: u64 = 16 * 1024;

/// The most provisional records a reader looks at for one session.
const MAX_PROVISIONAL_READ: usize = 64;

#[path = "crash_record_listing.rs"]
mod listing;
use listing::MAX_LISTED;

#[path = "crash_record_names.rs"]
mod names;
use names::{RecordName, decimal};

#[path = "crash_record_sweep.rs"]
mod sweep;
pub use sweep::STALE_RECORD_AGE;

/// The scope a fatal record falls back to when a directory someone planted
/// under `<digest>.crash` cannot be removed (#2192 review): no call's scope is
/// 0 (they count from 1), so no call's end ever withdraws it.
const FATAL_FALLBACK_SCOPE: u64 = 0;

/// Mixed into a temporary file's name when no random bytes can be had.
static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

/// How many temporary names a write tries before giving up: a name is taken
/// only when someone else already made a file under it.
const TEMPORARY_ATTEMPTS: usize = 8;

/// The crash directory, held open; every record is reached through it.
#[derive(Debug)]
pub struct RecordDir {
    dir: std::fs::File,
}

impl RecordDir {
    /// The crash directory under `base_dir`, created private (0700) with
    /// the audit directory it is in, each opened without following a link.
    pub fn create(base_dir: &Path) -> std::io::Result<Self> {
        use std::os::unix::fs::DirBuilderExt;
        let audit = AuditLog::directory(base_dir);
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&audit)?;
        let audit = Self::open(&audit)?;
        let crash = c_name(AuditLog::CRASH_DIRECTORY)?;
        // SAFETY: `audit.dir` is an open directory and `crash` is NUL-terminated.
        let made = unsafe { libc::mkdirat(audit.dir.as_raw_fd(), crash.as_ptr(), 0o700) };
        match made {
            0 => {}
            _ => match std::io::Error::last_os_error() {
                error if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                error => return Err(error),
            },
        }
        audit.subdirectory(&crash)
    }

    /// The crash directory under `base_dir`, for reading: nothing is
    /// created, and a directory that is a link is refused.
    pub fn existing(base_dir: &Path) -> std::io::Result<Self> {
        Self::open(&AuditLog::directory(base_dir))?
            .subdirectory(&c_name(AuditLog::CRASH_DIRECTORY)?)
    }

    fn open(path: &Path) -> std::io::Result<Self> {
        use std::os::unix::fs::OpenOptionsExt;
        let dir = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        Ok(Self { dir })
    }

    /// The directory `name` in this one, reached through it, not a link.
    fn subdirectory(&self, name: &CString) -> std::io::Result<Self> {
        let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        let dir = self.open_at(name, flags, 0)?;
        Ok(Self { dir })
    }

    /// Replace the file `name` with `record`, whole: written to a temporary
    /// file in the directory, then renamed over it.
    fn write(&self, name: &str, record: &CrashRecord) -> std::io::Result<()> {
        self.write_with(name, record, &mut unpredictable)
    }

    /// [`Self::write`], its temporary file named with suffixes from
    /// `suffix`: a name someone already made a file under is skipped for
    /// the next, [`TEMPORARY_ATTEMPTS`] times at most (#2192 review: a
    /// co-tenant cannot stop a record by pre-creating its name).
    fn write_with(
        &self,
        name: &str,
        record: &CrashRecord,
        suffix: &mut dyn FnMut() -> u64,
    ) -> std::io::Result<()> {
        use std::io::Write;
        let mut line = serde_json::to_string(record).map_err(std::io::Error::other)?;
        line.push('\n');
        let flags =
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        let mut taken = None;
        for _ in 0..TEMPORARY_ATTEMPTS {
            let temporary = c_name(&format!(".{name}.{:016x}.tmp", suffix()))?;
            match self.open_at(&temporary, flags, 0o600) {
                Ok(file) => {
                    taken = Some((temporary, file));
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        let Some((temporary, mut file)) = taken else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "every temporary name tried was already taken",
            ));
        };
        let written = file.write_all(line.as_bytes());
        drop(file);
        match written.and_then(|()| self.rename_into_place(&temporary, &c_name(name)?)) {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = self.unlink(&temporary, 0);
                Err(error)
            }
        }
    }

    /// The record in the file `name`, if a regular file of at most
    /// [`MAX_RECORD_BYTES`] holds one, its texts bounded again (the file
    /// may have been written by anyone).
    fn read(&self, name: &str) -> Option<CrashRecord> {
        use std::io::Read;
        let flags = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC;
        let file = self.open_at(&c_name(name).ok()?, flags, 0).ok()?;
        let metadata = file.metadata().ok()?;
        let readable = metadata.file_type().is_file() && metadata.len() <= MAX_RECORD_BYTES;
        let mut text = String::new();
        match readable {
            true => file.take(MAX_RECORD_BYTES).read_to_string(&mut text).ok()?,
            false => return None,
        };
        serde_json::from_str::<CrashRecord>(text.trim_end())
            .ok()
            .map(CrashRecord::bounded)
    }

    /// Remove the file `name`; one that is not there is already removed.
    /// A directory planted under the name is removed too when it is empty
    /// (#2192 review); a non-empty one is an error. What the entry is, is
    /// asked of it (`fstatat`, not following a link), not guessed from the
    /// error an unlink gives — which differs by platform (`EISDIR` on
    /// Linux, `EPERM` on macOS).
    fn remove(&self, name: &str) -> std::io::Result<()> {
        let name = c_name(name)?;
        let flags = match self.entry_mode(&name) {
            Ok(mode) => unlink_flags(mode),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        match self.unlink(&name, flags) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }

    /// Whether the entry `name` is a directory (not following a link).
    fn is_directory(&self, name: &str) -> bool {
        c_name(name)
            .and_then(|name| self.entry_mode(&name))
            .is_ok_and(|mode| unlink_flags(mode) == libc::AT_REMOVEDIR)
    }

    /// The `st_mode` of the entry `name`, not following a link.
    fn entry_mode(&self, name: &CString) -> std::io::Result<libc::mode_t> {
        self.entry_stat(name).map(|stat| stat.st_mode)
    }

    /// The status of the entry `name`, not following a link.
    fn entry_stat(&self, name: &CString) -> std::io::Result<libc::stat> {
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        // fstatat writes only into the `stat` it is given.
        // SAFETY: `name` is NUL-terminated and `dir` is open.
        let status = unsafe {
            libc::fstatat(
                self.dir.as_raw_fd(),
                name.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        match status {
            // SAFETY: fstatat succeeded, so it filled `stat` in.
            0 => Ok(unsafe { stat.assume_init() }),
            _ => Err(std::io::Error::last_os_error()),
        }
    }

    /// The names in the directory that start with `prefix`, sorted, from
    /// at most [`MAX_LISTED`] entries; a listing cut short is logged.
    fn names_with_prefix(&self, prefix: &str) -> Vec<String> {
        self.names_matching(prefix, &|name| name.starts_with(prefix))
    }

    /// The names in the directory `keep` keeps, sorted, from at most
    /// [`MAX_LISTED`] entries; a listing cut short is logged, as `what`.
    fn names_matching(&self, what: &str, keep: &dyn Fn(&str) -> bool) -> Vec<String> {
        let listed = listing::names_matching_within(self, keep, MAX_LISTED);
        match listed.complete {
            true => {}
            false => tracing::warn!(
                what,
                found = listed.names.len(),
                "the crash record directory was not listed to its end: records may be missed"
            ),
        }
        listed.names
    }

    /// The names in the held directory that start with `prefix`, from at
    /// most `examined` entries ([`listing::names_matching_within`]).
    #[cfg(test)]
    fn names_with_prefix_within(&self, prefix: &str, examined: usize) -> listing::Listed {
        listing::names_matching_within(self, &|name| name.starts_with(prefix), examined)
    }

    fn open_at(
        &self,
        name: &CString,
        flags: libc::c_int,
        mode: libc::c_uint,
    ) -> std::io::Result<std::fs::File> {
        // SAFETY: `dir` is an open directory for the call and `name` is NUL-terminated.
        let fd = unsafe { libc::openat(self.dir.as_raw_fd(), name.as_ptr(), flags, mode) };
        match fd {
            fd if fd >= 0 => {
                // SAFETY: `fd` was just opened and is owned by nothing else.
                Ok(unsafe { std::fs::File::from_raw_fd(fd) })
            }
            _ => Err(std::io::Error::last_os_error()),
        }
    }

    fn rename_into_place(&self, from: &CString, to: &CString) -> std::io::Result<()> {
        let dir = self.dir.as_raw_fd();
        // SAFETY: both names are NUL-terminated and relative to the open `dir`.
        let status = unsafe { libc::renameat(dir, from.as_ptr(), dir, to.as_ptr()) };
        match status {
            0 => Ok(()),
            _ => Err(std::io::Error::last_os_error()),
        }
    }

    fn unlink(&self, name: &CString, flags: libc::c_int) -> std::io::Result<()> {
        // SAFETY: `name` is NUL-terminated and relative to the open `dir`.
        let status = unsafe { libc::unlinkat(self.dir.as_raw_fd(), name.as_ptr(), flags) };
        match status {
            0 => Ok(()),
            _ => Err(std::io::Error::last_os_error()),
        }
    }
}

/// An unpredictable 64-bit value for a temporary file's name: random bytes
/// from the system, else (none to be had) the time, pid, a serial and a
/// stack address mixed. Never panics: a panic hook calls it.
fn unpredictable() -> u64 {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        let mut value = 0u64;
        // SAFETY: getrandom writes at most 8 bytes into `value`, 8 bytes long.
        let got = unsafe {
            libc::getrandom(
                std::ptr::addr_of_mut!(value).cast(),
                std::mem::size_of::<u64>(),
                libc::GRND_NONBLOCK,
            )
        };
        if got == 8 {
            return value;
        }
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_nanos() as u64)
        .unwrap_or_default();
    let serial = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
    let here = std::ptr::addr_of!(serial) as u64;
    nanos
        ^ (u64::from(std::process::id()) << 32)
        ^ serial.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ here
}

fn c_name(name: &str) -> std::io::Result<CString> {
    CString::new(name)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "a NUL in the name"))
}

/// How an entry of `mode` is unlinked: a directory with `AT_REMOVEDIR`,
/// anything else as a file.
fn unlink_flags(mode: libc::mode_t) -> libc::c_int {
    match mode & libc::S_IFMT {
        libc::S_IFDIR => libc::AT_REMOVEDIR,
        _ => 0,
    }
}

/// The pid and scope a provisional record's name ends with — `rest`, what
/// follows its prefix: `<scope>` when the prefix already names `writer`'s
/// pid, `<pid>.<scope>` when it names none. `None` for any other form.
fn provisional_ids(rest: &str, writer: Option<u32>) -> Option<(u64, u64)> {
    match writer {
        Some(pid) => Some((u64::from(pid), decimal(rest)?)),
        None => {
            let (pid, scope) = rest.split_once('.')?;
            Some((decimal(pid)?, decimal(scope)?))
        }
    }
}

/// The record files of one session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecords {
    key: String,
    /// The key's digest, every record name's start.
    stem: String,
    fatal: String,
    provisional_prefix: String,
}

impl SessionRecords {
    pub fn new(session_key: &str) -> Self {
        assert!(!session_key.is_empty(), "a crash record names its session");
        let fatal = AuditLog::crash_record_name(session_key);
        let stem = fatal
            .strip_suffix(".crash")
            .expect("a fatal record's name ends `.crash`")
            .to_string();
        assert!(
            RecordName::parse(&fatal) == Some(RecordName::Fatal { stem: &stem }),
            "a session's fatal record name has the fatal shape"
        );
        Self {
            key: session_key.to_string(),
            stem,
            provisional_prefix: format!("{fatal}.provisional."),
            fatal,
        }
    }

    /// The session these records are of.
    pub fn key(&self) -> &str {
        &self.key
    }

    fn provisional(&self, scope: u64) -> String {
        format!("{}{scope}", self.provisional_prefix_of(std::process::id()))
    }

    /// The start of every provisional record name the process `pid` wrote.
    fn provisional_prefix_of(&self, pid: u32) -> String {
        format!("{}{pid}.", self.provisional_prefix)
    }

    /// Write the fatal record: the only writer of `<digest>.crash`. A
    /// directory someone planted under that name (#2192 review) is removed
    /// when empty; a non-empty one leaves the record under this process's
    /// fallback name ([`FATAL_FALLBACK_SCOPE`]), which a reader of this
    /// process finds as it finds a provisional one — the record itself says
    /// it is not provisional.
    pub fn write_fatal(&self, dir: &RecordDir, record: &CrashRecord) -> std::io::Result<()> {
        let record = &record.clone().for_session(&self.key);
        match dir.write(&self.fatal, record) {
            Err(_) if dir.is_directory(&self.fatal) => match dir.remove(&self.fatal) {
                Ok(()) => dir.write(&self.fatal, record),
                Err(_) => dir.write(&self.provisional(FATAL_FALLBACK_SCOPE), record),
            },
            other => other,
        }
    }

    /// Write the provisional record of the call scoped `scope`.
    pub fn write_provisional(
        &self,
        dir: &RecordDir,
        scope: u64,
        record: &CrashRecord,
    ) -> std::io::Result<()> {
        dir.write(
            &self.provisional(scope),
            &record.clone().for_session(&self.key),
        )
    }

    /// Remove the provisional record of `scope`, and nothing else.
    pub fn withdraw(&self, dir: &RecordDir, scope: u64) -> std::io::Result<()> {
        dir.remove(&self.provisional(scope))
    }

    /// The session's record, as the process `writer` left it (#2192
    /// review): the fatal one if it says that process's pid, else the
    /// newest provisional one named for that pid that says the same pid.
    /// With no process expected (`None`: the reader knows none, and
    /// believes nothing it reads), the fatal one, else the newest of any.
    /// Either way a record is taken only if it names this session's exact
    /// key (#2192 review): one naming another session, or none, is not
    /// this session's, whatever name it is under.
    ///
    /// The pid is the record's own word: this only keeps one process's
    /// records from being mistaken for another's by accident or by a
    /// careless writer. Anyone who can write the directory (this user, in
    /// the same-uid trust model) can forge a record naming the expected pid.
    ///
    /// Only names of the exact form `<digest>.crash.provisional.<pid>.<scope>`
    /// (decimal digits) are read, the newest scopes first and at most
    /// [`MAX_PROVISIONAL_READ`] of them: a name of any other form (a
    /// squatter's) is skipped before the bound, and skipping is logged.
    pub fn read(&self, dir: &RecordDir, writer: Option<u32>) -> Option<CrashRecord> {
        let written_by = |record: &CrashRecord| match (record.is_for_session(&self.key), writer) {
            (true, Some(pid)) => record.pid == pid,
            (true, None) => true,
            (false, _) => false,
        };
        let prefix = match writer {
            Some(pid) => self.provisional_prefix_of(pid),
            None => self.provisional_prefix.clone(),
        };
        dir.read(&self.fatal).filter(written_by).or_else(|| {
            let listed = dir.names_with_prefix(&prefix);
            let mut named: Vec<((u64, u64), String)> = listed
                .iter()
                .filter_map(|name| {
                    provisional_ids(&name[prefix.len()..], writer).map(|ids| (ids, name.clone()))
                })
                .collect();
            let (malformed, over) = (
                listed.len() - named.len(),
                named.len().saturating_sub(MAX_PROVISIONAL_READ),
            );
            match (malformed, over) {
                (0, 0) => {}
                _ => tracing::warn!(
                    session = %self.key,
                    malformed,
                    over,
                    "provisional crash record names were skipped"
                ),
            }
            named.sort_unstable_by_key(|(ids, _)| std::cmp::Reverse(*ids));
            named
                .into_iter()
                .take(MAX_PROVISIONAL_READ)
                .filter_map(|(_, name)| dir.read(&name))
                .filter(written_by)
                .max_by_key(|record| record.unix_ms)
        })
    }

    /// Remove every record of the session: those an earlier run left, and
    /// the temporary files of its writes that never reached their rename
    /// (#2192 review: a process killed mid-write leaves one). Only names
    /// of a record's exact shapes, of this session's digest, are removed.
    pub fn clear(&self, dir: &RecordDir) {
        let own = |name: &str| match RecordName::parse(name) {
            Some(RecordName::Fatal { .. }) => false,
            Some(record) => record.stem() == self.stem,
            None => false,
        };
        let stale = std::iter::once(self.fatal.clone()).chain(dir.names_matching(&self.key, &own));
        for name in stale {
            if let Err(error) = dir.remove(&name) {
                tracing::warn!(%error, name, "could not remove a stale crash record");
            }
        }
    }
}

#[path = "crash_record_target.rs"]
mod target;
pub use target::{
    Armed, CrashTarget, arm_prepared, claimed, follow, prepare, record_fatal, record_provisional,
    withdraw,
};

#[cfg(test)]
#[path = "crash_record_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "crash_record_clear_tests.rs"]
mod clear_tests;
