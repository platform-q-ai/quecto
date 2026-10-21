//! The authority launcher (#2615, ADR-0032): starts
//! `quecto admission-broker run --stop-when-idle` detached when a root's
//! authority does not answer. Whether to start one is
//! `NegotiateAuthority::ensure_running`'s decision; this adapter probes the
//! client socket, starts the broker through the detached-process adapter
//! with its output in `broker.log` in the admission directory (outside
//! `client/`), reports how it ended, and paces probes with the link's
//! reconnect backoff.
use std::path::{Path, PathBuf};

use super::link::Backoff;
use crate::application::admission::ports::{
    AuthorityLaunch, AuthorityLauncher, LaunchState, LaunchedAuthority,
};
use crate::infrastructure::processes::local::detached_process::{DetachedProcess, Tether};

/// The only program this launcher starts: the quecto CLI itself.
const BROKER_PROGRAM: &str = "quecto";
/// The broker's log, in the admission directory.
const LOG_FILE: &str = "broker.log";
/// `admission-broker run`'s exit status when another broker holds the lock.
const BUSY_EXIT: i32 = 3;

/// Starts brokers from the quecto binary at `program`.
#[derive(Debug, Clone)]
pub struct DetachedAuthorityLauncher {
    program: Result<PathBuf, String>,
    tether: Tether,
}

impl DetachedAuthorityLauncher {
    /// Start brokers from `program`.
    pub fn new(program: PathBuf) -> Self {
        Self {
            program: Ok(program),
            tether: Tether::Detached,
        }
    }

    /// Start brokers from the running binary; when it cannot be resolved,
    /// every launch says why.
    pub fn for_current_exe() -> Self {
        Self {
            program: std::env::current_exe()
                .map_err(|error| format!("cannot resolve the quecto binary: {error}")),
            tether: Tether::Detached,
        }
    }

    /// Start brokers that die with the thread that started them (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn dying_with_launching_thread(program: PathBuf) -> Self {
        Self {
            program: Ok(program),
            tether: Tether::DiesWithLaunchingThread,
        }
    }

    /// Where a broker started for `directory` logs.
    pub fn log_path(directory: &Path) -> PathBuf {
        directory.join(LOG_FILE)
    }
}

impl AuthorityLauncher for DetachedAuthorityLauncher {
    fn probe(&self, directory: &Path) -> Result<(), String> {
        let _ = directory;
        Err("not implemented".into())
    }

    fn launch(&self, launch: &AuthorityLaunch) -> Result<Box<dyn LaunchedAuthority>, String> {
        let _ = (
            launch,
            &self.program,
            self.tether,
            BROKER_PROGRAM,
            BUSY_EXIT,
        );
        Err("not implemented".into())
    }

    fn pause(&self, attempt: u32) {
        std::thread::sleep(Backoff::RECONNECT.delay(attempt));
    }
}

/// A broker this launcher started.
pub struct LaunchedBroker {
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
