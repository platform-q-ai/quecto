//! Capability-local ports of the admission operation capability (#2024 S3).
//! Infrastructure implements them; the use cases never open a socket, run a
//! process or touch a filesystem themselves. Connection and reconnect stay
//! behind the existing admission client port in `infrastructure/admission`.

use std::path::{Path, PathBuf};

use super::dto::{AuthorityAdminError, AuthorityReport, ResetReport};

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
}
