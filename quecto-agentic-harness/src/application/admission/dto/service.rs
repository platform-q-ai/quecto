//! Installing and uninstalling the broker as a systemd user service: the
//! request, each action taken and the report.

use std::path::PathBuf;

/// Everything needed to install the broker as a systemd user service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallServiceRequest {
    /// Absolute path to the `quecto` binary the unit runs.
    pub binary: PathBuf,
    /// Absolute config path the unit's `run` addresses.
    pub config: PathBuf,
    /// The authority directory the addressed config resolves to (for output).
    pub directory: PathBuf,
    /// Report the intended actions without touching systemd or the unit file.
    pub dry_run: bool,
}

/// One thing an install or uninstall did (or would do), reported verbatim so
/// the operator sees exactly what happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceAction {
    WroteUnit {
        path: PathBuf,
    },
    UnitUnchanged {
        path: PathBuf,
    },
    RemovedUnit {
        path: PathBuf,
    },
    NoUnitToRemove {
        path: PathBuf,
    },
    DaemonReloaded,
    EnabledAndStarted {
        unit: String,
    },
    /// A changed unit was rewritten and the running service restarted on it.
    Restarted {
        unit: String,
    },
    DisabledAndStopped {
        unit: String,
    },
    NothingToDisable {
        unit: String,
    },
    /// A dry run: the planned action, not one performed.
    Planned {
        description: String,
    },
}

/// The result of an install/uninstall, tagged with the directory addressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceReport {
    pub directory: PathBuf,
    pub unit: String,
    pub unit_path: PathBuf,
    pub dry_run: bool,
    pub actions: Vec<ServiceAction>,
}
