//! Start a process detached from this one (#2615): the effect behind the
//! admission broker started on demand. The child leads a session (and so a
//! process group) of its own, so nothing sent to the launcher's terminal or
//! group reaches it; when a systemd user manager is reachable it also runs in
//! a transient scope of its own (`systemd-run --user --scope`), so stopping
//! the launching terminal's unit does not stop it. It gets exactly the
//! environment it is given, reads nothing, appends its stdout and stderr to
//! a log, and runs from `/` so it never holds a working directory busy. It
//! is this process's child only until this process exits, and outlives it.
//! Which program to start, with which environment, and when, is the caller's.
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use crate::infrastructure::processes::parent_death_signal::own_session;
#[cfg(any(test, feature = "test-support"))]
use crate::infrastructure::processes::parent_death_signal::own_session_dying_with_us;

/// A log past this size is started afresh, at its open and while the
/// process writing it runs ([`cap_log`]), so it never grows without bound.
pub const LOG_CAP_BYTES: u64 = 1 << 20;

/// How long the transient-scope probe may take before the plain start is
/// used instead.
const SCOPE_PROBE_BOUND: Duration = Duration::from_secs(3);

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

/// Whether to leave the launcher's cgroup through a systemd transient scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// Start through this `systemd-run` when a user manager answers it;
    /// otherwise start plainly.
    SystemdRun(PathBuf),
    /// Start plainly (in the launcher's cgroup, its own session).
    None,
}

/// How a detached process was started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Started {
    /// In a transient systemd scope of its own.
    InScope,
    /// Plainly: no user manager answered (or none was asked).
    Plain,
}

/// What to start.
#[derive(Debug, Clone)]
pub struct DetachedCommand {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    /// Its whole environment: nothing else of this process's is passed on.
    pub env: Vec<(OsString, OsString)>,
    /// Where its stdout and stderr go (created owner-only).
    pub log: PathBuf,
    pub tether: Tether,
    pub scope: Scope,
    /// Text the program writes to its log whenever it runs at all: its
    /// absence after a failed start through the runner means the runner
    /// failed, and the program is started plainly instead.
    pub ran_marker: String,
}

/// Start `command` detached.
///
/// Through the scope runner when one answers; if the runner then fails
/// before the program ran (it exits unsuccessfully within
/// [`RUNNER_SETTLE`] and the log gained nothing naming the program's
/// [`DetachedCommand::ran_marker`]), the program is started plainly once.
///
/// # Errors
/// The log cannot be opened, or the process cannot be started.
pub fn spawn_detached(command: &DetachedCommand) -> Result<DetachedProcess, String> {
    let scoped = match &command.scope {
        Scope::SystemdRun(runner) if scope_available(runner, &command.env) => Some(runner),
        Scope::SystemdRun(_) | Scope::None => None,
    };
    let Some(runner) = scoped else {
        return start(command, None);
    };
    let before = std::fs::metadata(&command.log).map_or(0, |meta| meta.len());
    let mut process = start(command, Some(runner))?;
    match runner_failed(&mut process, &command.log, before, &command.ran_marker) {
        true => {
            drop(process);
            start(command, None)
        }
        false => Ok(process),
    }
}

/// How long a start through the scope runner is watched for the runner
/// failing before it exec'd the program.
pub const RUNNER_SETTLE: Duration = Duration::from_millis(300);

/// Whether a start through the runner ended, unsuccessfully, within
/// [`RUNNER_SETTLE`], with no sign in the log (past `before`) that the
/// program itself ran.
fn runner_failed(process: &mut DetachedProcess, log: &Path, before: u64, marker: &str) -> bool {
    let deadline = Instant::now() + RUNNER_SETTLE;
    let exit = loop {
        match process.exit() {
            Some(exit) => break exit,
            None if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            None => return false,
        }
    };
    if exit.as_ref().is_ok_and(ExitStatus::success) {
        return false;
    }
    let written = std::fs::read(log).unwrap_or_default();
    let since = written
        .get(usize::try_from(before).unwrap_or(usize::MAX)..)
        .unwrap_or(&[]);
    !String::from_utf8_lossy(since).contains(marker)
}

/// One start: through `runner` into a transient scope, or plainly.
fn start(command: &DetachedCommand, runner: Option<&PathBuf>) -> Result<DetachedProcess, String> {
    let log = open_log(&command.log)?;
    let log_err = log
        .try_clone()
        .map_err(|error| format!("log {}: {error}", command.log.display()))?;
    let (mut process, started) = match runner {
        Some(runner) => {
            let mut process = Command::new(runner);
            process.args(SCOPE_ARGS).arg(&command.program);
            (process, Started::InScope)
        }
        None => (Command::new(&command.program), Started::Plain),
    };
    process
        .args(&command.args)
        .env_clear()
        .envs(command.env.iter().map(|(key, value)| (key, value)))
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(log)
        .stderr(log_err);
    match command.tether {
        Tether::Detached => own_session(&mut process),
        #[cfg(any(test, feature = "test-support"))]
        Tether::DiesWithLaunchingThread => own_session_dying_with_us(&mut process),
    }
    // A runner that cannot start is named, not the program it would run.
    let starting = runner.unwrap_or(&command.program);
    let child = process
        .spawn()
        .map_err(|error| format!("cannot start {}: {error}", starting.display()))?;
    Ok(DetachedProcess {
        child: Some(child),
        exited: None,
        started,
    })
}

/// `systemd-run` arguments before the program: a transient scope of the
/// user manager, collected once it ends; the program is exec'd in place.
const SCOPE_ARGS: [&str; 5] = ["--user", "--scope", "--quiet", "--collect", "--"];

/// Whether the user manager starts a transient scope through `runner`:
/// `true` run in one, within [`SCOPE_PROBE_BOUND`]. Anything else (no such
/// program, no user manager, a slow bus) is the plain start.
fn scope_available(runner: &Path, env: &[(OsString, OsString)]) -> bool {
    let probe = Command::new(runner)
        .args(SCOPE_ARGS)
        .arg("true")
        .env_clear()
        .envs(env.iter().map(|(key, value)| (key, value)))
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let Ok(child) = probe else {
        return false;
    };
    let mut probe = DetachedProcess {
        child: Some(child),
        exited: None,
        started: Started::Plain,
    };
    let deadline = Instant::now() + SCOPE_PROBE_BOUND;
    loop {
        match probe.exit() {
            Some(Ok(status)) => return status.success(),
            Some(Err(_)) => return false,
            None if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            // Too slow: left to its reaper, and the plain start is used.
            None => return false,
        }
    }
}

/// Open `path` owner-only for appending; past [`LOG_CAP_BYTES`] it starts
/// afresh. Always in append mode, so a later [`cap_log`] that empties it
/// leaves every writer appending at its new end.
fn open_log(path: &Path) -> Result<File, String> {
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| format!("log {}: {error}", path.display()))?;
    cap_log(&log, LOG_CAP_BYTES).map_err(|error| format!("log {}: {error}", path.display()))?;
    Ok(log)
}

/// Empty `log` when it is a regular file past `cap` (a long-running
/// process's own log, #2615); `true` when it did. Anything that is not a
/// regular file (a terminal, a pipe) is left alone.
///
/// # Errors
/// The file's size cannot be read or set.
pub fn cap_log(log: &File, cap: u64) -> std::io::Result<bool> {
    let meta = log.metadata()?;
    match meta.is_file() && meta.len() > cap {
        true => log.set_len(0).map(|()| true),
        false => Ok(false),
    }
}

/// This process's own stderr as a file: what a detached process whose
/// stderr is its log keeps capped with [`cap_log`].
///
/// # Errors
/// Stderr cannot be duplicated.
pub fn own_stderr() -> std::io::Result<File> {
    use std::os::fd::AsFd;
    std::io::stderr()
        .as_fd()
        .try_clone_to_owned()
        .map(File::from)
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
    started: Started,
}

impl DetachedProcess {
    /// How it was started.
    pub fn started(&self) -> Started {
        self.started
    }

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
