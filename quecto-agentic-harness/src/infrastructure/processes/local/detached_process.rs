//! Start a process detached from this one (#2615): the effect behind the
//! admission broker started on demand. The child leads a session (and so a
//! process group) of its own, so nothing sent to the launcher's terminal or
//! group reaches it; it reads nothing; its stdout and stderr append to a
//! log; and it runs from `/` so it never holds a working directory busy.
//! It is this process's child only until this process exits, and outlives
//! it. Which program to start, and when, is the caller's.
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus};

/// A log past this size is started afresh rather than appended to, so
/// repeated starts never grow it without bound.
pub const LOG_CAP_BYTES: u64 = 1 << 20;

/// How the detached process relates to the thread that started it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tether {
    /// Production: it outlives the launcher.
    Detached,
    /// Tests: it is sent SIGTERM when the launching thread ends, so nothing
    /// a test starts outlives it (#2631).
    #[cfg(any(test, feature = "test-support"))]
    DiesWithLaunchingThread,
}

/// What to start.
#[derive(Debug, Clone)]
pub struct DetachedCommand {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub env: Vec<(OsString, OsString)>,
    /// Where its stdout and stderr go (created owner-only).
    pub log: PathBuf,
    pub tether: Tether,
}

/// Start `command` detached.
///
/// # Errors
/// The log cannot be opened, or the process cannot be started.
pub fn spawn_detached(command: &DetachedCommand) -> Result<DetachedProcess, String> {
    let _ = (command, open_log);
    Err("not implemented".into())
}

/// Open `path` owner-only for appending; past [`LOG_CAP_BYTES`] it starts
/// afresh.
fn open_log(path: &Path) -> Result<File, String> {
    let oversized = std::fs::metadata(path).is_ok_and(|meta| meta.len() > LOG_CAP_BYTES);
    let mut options = OpenOptions::new();
    options.create(true).mode(0o600);
    match oversized {
        true => options.write(true).truncate(true),
        false => options.append(true),
    };
    options
        .open(path)
        .map_err(|error| format!("log {}: {error}", path.display()))
}

/// How a detached process ended.
pub type Exit = Result<ExitStatus, String>;

/// A process [`spawn_detached`] started.
#[derive(Debug)]
pub struct DetachedProcess {
    /// Held until it exits, then reaped; handed to a reaper on drop.
    child: Option<Child>,
    /// How it ended, once it has.
    exited: Option<Exit>,
}

impl DetachedProcess {
    /// `None` while it runs; how it ended once it has (kept from then on).
    pub fn exit(&mut self) -> Option<Exit> {
        if let Some(exited) = &self.exited {
            return Some(exited.clone());
        }
        let child = self.child.as_mut()?;
        let exited = match child.try_wait() {
            Ok(None) => return None,
            Ok(Some(status)) => Ok(status),
            Err(error) => Err(error.to_string()),
        };
        self.child = None;
        self.exited = Some(exited.clone());
        Some(exited)
    }
}

impl Drop for DetachedProcess {
    /// A process still running outlives this handle; a reaper waits for it
    /// so it never lingers as a zombie of this process.
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        let reaper = std::thread::Builder::new()
            .name("quecto-detached-reaper".into())
            .spawn(move || {
                let _ = child.wait();
            });
        if let Err(error) = reaper {
            tracing::warn!(%error, "no reaper for a detached process; it is reaped at exit");
        }
    }
}

#[cfg(test)]
#[path = "detached_process_tests.rs"]
mod tests;
