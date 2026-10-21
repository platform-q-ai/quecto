//! The authority launcher (#2615, ADR-0032): starts
//! `quecto admission-broker run --stop-when-idle` detached when a root's
//! authority does not answer. Whether to start one is
//! `NegotiateAuthority::ensure_running`'s decision; this adapter probes the
//! client socket, starts the broker through the detached-process adapter
//! with its output in `broker.log` in the admission directory (outside
//! `client/`), reports how it ended, and paces probes with the link's
//! reconnect backoff.
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::directory::AuthorityDirectory;
use super::link::Backoff;
use crate::application::admission::ports::{
    AuthorityLaunch, AuthorityLauncher, LaunchState, LaunchedAuthority,
};
use crate::infrastructure::processes::local::detached_process::{
    DetachedCommand, DetachedProcess, Scope, Tether, spawn_detached,
};

/// The only program this launcher starts: the quecto CLI itself.
const BROKER_PROGRAM: &str = "quecto";
/// The broker's log, in the admission directory.
const LOG_FILE: &str = "broker.log";
/// Resolved on the broker's `PATH`; absent or without a user manager, the
/// broker starts plainly in its own session.
const SYSTEMD_RUN: &str = "systemd-run";
/// `admission-broker run`'s exit status when another broker holds the lock.
const BUSY_EXIT: i32 = 3;

/// The variables a broker inherits from its launcher, by exact name; `LC_*`
/// is matched by prefix. Nothing else reaches it: no provider keys or
/// tokens (readable in its `/proc/<pid>/environ` for as long as it runs),
/// and no `RUST_LOG`, which it gets fixed at [`BROKER_LOG_LEVEL`]. `PATH`,
/// `XDG_RUNTIME_DIR` and `DBUS_SESSION_BUS_ADDRESS` also let
/// `systemd-run --user` reach the user manager.
const INHERITED: &[&str] = &[
    "PATH",
    "HOME",
    "XDG_RUNTIME_DIR",
    "XDG_CONFIG_HOME",
    "DBUS_SESSION_BUS_ADDRESS",
    "LANG",
    "TZ",
];
/// The broker logs warnings only, so its log grows on transitions alone.
const BROKER_LOG_LEVEL: &str = "warn";

/// The broker's whole environment: the allowlisted variables of `parent`,
/// `QUECTO_BASE_DIR` set to the launch's base directory, and `RUST_LOG`.
pub fn broker_environment(
    parent: impl IntoIterator<Item = (OsString, OsString)>,
    base_dir: &Path,
) -> Vec<(OsString, OsString)> {
    let mut env: Vec<(OsString, OsString)> = parent
        .into_iter()
        .filter(|(name, _)| {
            name.to_str()
                .is_some_and(|name| INHERITED.contains(&name) || name.starts_with("LC_"))
        })
        .collect();
    env.push(("QUECTO_BASE_DIR".into(), base_dir.as_os_str().to_owned()));
    env.push(("RUST_LOG".into(), BROKER_LOG_LEVEL.into()));
    env
}

/// Starts brokers from the quecto binary at `program`.
#[derive(Debug, Clone)]
pub struct DetachedAuthorityLauncher {
    program: Result<PathBuf, String>,
    tether: Tether,
    scope: Scope,
}

impl DetachedAuthorityLauncher {
    /// Start brokers from `program`.
    pub fn new(program: PathBuf) -> Self {
        Self {
            program: Ok(program),
            tether: Tether::Detached,
            scope: Scope::SystemdRun(PathBuf::from(SYSTEMD_RUN)),
        }
    }

    /// Start brokers from the running binary; when it cannot be resolved,
    /// every launch says why.
    pub fn for_current_exe() -> Self {
        Self {
            program: std::env::current_exe()
                .map_err(|error| format!("cannot resolve the quecto binary: {error}")),
            tether: Tether::Detached,
            scope: Scope::SystemdRun(PathBuf::from(SYSTEMD_RUN)),
        }
    }

    /// Start brokers that die with the thread that started them (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn dying_with_launching_thread(program: PathBuf) -> Self {
        Self {
            program: Ok(program),
            tether: Tether::DiesWithLaunchingThread,
            scope: Scope::None,
        }
    }

    /// Start plainly, never through `systemd-run` (tests: a fake broker
    /// never touches the real user manager).
    #[cfg(any(test, feature = "test-support"))]
    pub fn without_scope(mut self) -> Self {
        self.scope = Scope::None;
        self
    }

    /// Leave the launcher's cgroup through `runner` instead of `systemd-run`
    /// (tests stand a script in for it).
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_scope_runner(mut self, runner: PathBuf) -> Self {
        self.scope = Scope::SystemdRun(runner);
        self
    }

    /// Where a broker started for `directory` logs.
    pub fn log_path(directory: &Path) -> PathBuf {
        directory.join(LOG_FILE)
    }
}

impl AuthorityLauncher for DetachedAuthorityLauncher {
    fn probe(&self, directory: &Path) -> Result<(), String> {
        let socket = AuthorityDirectory::from_directory(directory).client_socket();
        std::os::unix::net::UnixStream::connect(&socket)
            .map(drop)
            .map_err(|error| format!("connect {}: {error}", socket.display()))
    }

    fn launch(&self, launch: &AuthorityLaunch) -> Result<Box<dyn LaunchedAuthority>, String> {
        let program = self.program.clone()?;
        // Affirmative guard: only the quecto CLI is ever started, so a
        // harness embedded in another binary (a test binary, say) never
        // starts itself with the broker's arguments.
        match program.file_name().and_then(|name| name.to_str()) {
            Some(BROKER_PROGRAM) => {}
            _ => {
                return Err(format!(
                    "{} is not the quecto binary; start `quecto admission-broker run` yourself",
                    program.display()
                ));
            }
        }
        // The broker's own private directory, created owner-only (or
        // refused when it is shared) before its log is opened in it.
        let directory = AuthorityDirectory::open(&launch.directory)
            .map_err(|error| format!("directory {}: {error}", launch.directory.display()))?;
        let log = Self::log_path(directory.path());
        let args = [
            OsString::from("admission-broker"),
            "run".into(),
            "--stop-when-idle".into(),
            "--config".into(),
            launch.config.clone().into(),
            "--directory".into(),
            launch.directory.clone().into(),
        ];
        let process = spawn_detached(&DetachedCommand {
            program,
            args: args.to_vec(),
            env: broker_environment(std::env::vars_os(), &launch.base_dir),
            log: log.clone(),
            tether: self.tether,
            scope: self.scope.clone(),
        })?;
        Ok(Box::new(LaunchedBroker { process, log }))
    }

    fn pause(&self, attempt: u32) {
        std::thread::sleep(Backoff::RECONNECT.delay(attempt));
    }
}

/// A broker this launcher started.
struct LaunchedBroker {
    process: DetachedProcess,
    log: PathBuf,
}

impl LaunchedAuthority for LaunchedBroker {
    fn state(&mut self) -> LaunchState {
        match self.process.exit() {
            None => LaunchState::Running,
            Some(Ok(status)) if status.code() == Some(BUSY_EXIT) => LaunchState::LostRace,
            Some(Ok(status)) => LaunchState::Failed {
                reason: format!(
                    "the admission broker exited ({status}) before it answered; see {}",
                    self.log.display()
                ),
            },
            Some(Err(error)) => LaunchState::Failed {
                reason: format!(
                    "the admission broker cannot be observed ({error}); see {}",
                    self.log.display()
                ),
            },
        }
    }
}

#[cfg(test)]
#[path = "on_demand_launcher_tests.rs"]
mod tests;
