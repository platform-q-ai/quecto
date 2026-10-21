//! Capability-local ports of the admission operation capability (#2024 S3).
//! Infrastructure implements them; the use cases never open a socket, run a
//! process or touch a filesystem themselves. An agent's connection to the
//! broker and its reconnects stay in the protocol client in
//! `infrastructure/admission/client.rs`, behind `inference_attempt::AttemptAdmission`.
//!
//! The runtime authority's ports sit beside them: the admission service's
//! registry, client, dispatcher and recovery ports (`inference_admission`),
//! the per-attempt admission (`inference_attempt`), the authority's journal
//! and secret source (`inference_authority_ports`), and the read port for
//! admission activity (`inference_observation`).

use std::path::{Path, PathBuf};

use super::dto::authority::{AuthorityAdminError, AuthorityReport, ResetReport};

pub mod inference_admission;
pub mod inference_attempt;
pub mod inference_authority_ports;
pub mod inference_observation;

/// Owner-only administration of a running authority over its admin socket.
/// Every operation names the directory it addresses so nothing depends on the
/// working directory.
pub trait AuthorityAdmin: Send + Sync {
    /// Read the authority's state at `directory` (its `admin.sock`).
    fn inspect(&self, directory: &Path) -> Result<AuthorityReport, AuthorityAdminError>;
    /// Start a new accounting epoch at `directory`.
    fn reset(&self, directory: &Path) -> Result<ResetReport, AuthorityAdminError>;
}

/// The systemd user unit as the manager should persist it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceUnitSpec {
    /// The full unit file contents to write.
    pub contents: String,
}

/// What the manager found on disk for the unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnitStatus {
    Absent,
    Present { contents: String },
}

/// Manage the broker's systemd *user* unit and its lifecycle (#2024 S3). The
/// unit name is fixed; the adapter knows the unit path. `enable_now` /
/// `disable_now` are `systemctl --user` calls; the contract test fakes
/// `systemctl` on the PATH.
pub trait AuthorityServiceManager: Send + Sync {
    /// The absolute path the unit lives at (for reporting).
    fn unit_path(&self) -> PathBuf;
    /// The unit name (`quecto-admission-broker.service`).
    fn unit_name(&self) -> String;
    /// Read the currently installed unit, if any.
    fn unit_status(&self) -> Result<UnitStatus, String>;
    /// Write (or overwrite) the unit file atomically.
    fn write_unit(&self, spec: &ServiceUnitSpec) -> Result<(), String>;
    /// Remove the unit file; `Ok(false)` when it was already absent.
    fn remove_unit(&self) -> Result<bool, String>;
    /// `systemctl --user daemon-reload`.
    fn daemon_reload(&self) -> Result<(), String>;
    /// `systemctl --user enable --now <unit>`.
    fn enable_now(&self) -> Result<(), String>;
    /// `systemctl --user disable --now <unit>`; `Ok(false)` when the unit was
    /// not enabled/loaded (so uninstall stays idempotent).
    fn disable_now(&self) -> Result<bool, String>;
    /// `systemctl --user restart <unit>`: pick up a rewritten unit.
    fn restart(&self) -> Result<(), String>;
    /// `systemctl --user is-active <unit>`: whether its broker is running
    /// (#2615); a stopped, failed or unknown unit is `Ok(false)`.
    fn is_active(&self) -> Result<bool, String>;
}

/// What starting a broker on demand needs (#2615): the directory it must
/// serve, the global config whose `admission` section it serves, and the
/// base directory that config resolves against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityLaunch {
    pub directory: PathBuf,
    pub config: PathBuf,
    pub base_dir: PathBuf,
}

/// How a broker this process launched is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchState {
    /// Still running: starting, or serving.
    Running,
    /// It exited Busy: another broker holds the directory's singleton lock.
    LostRace,
    /// It exited for any other reason, which names where to look.
    Failed { reason: String },
}

/// Start the authority on demand (#2615, ADR-0032): probe its client
/// socket, launch a detached broker, and pause between probes. The
/// singleton lock settles concurrent launches; the adapter never decides
/// whether to launch, only how.
pub trait AuthorityLauncher: Send + Sync {
    /// `Ok` when a broker accepts connections at `directory`'s client
    /// socket; otherwise why not.
    fn probe(&self, directory: &Path) -> Result<(), String>;
    /// Launch a detached broker serving `launch`, stopping itself when idle.
    fn launch(&self, launch: &AuthorityLaunch) -> Result<Box<dyn LaunchedAuthority>, String>;
    /// Wait before probe `attempt` (0-based): the bounded, jittered backoff.
    fn pause(&self, attempt: u32);
}

/// A broker this process launched.
pub trait LaunchedAuthority: Send {
    /// How it is doing now.
    fn state(&mut self) -> LaunchState;
}
