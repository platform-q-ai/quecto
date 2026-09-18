//! Request and result types of the admission operation capability (#2024 S3).
//! These are the application's own vocabulary; the interface maps CLI/UDS
//! shapes to and from them and never leaks a wire or systemd type inward.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// A quota group's operator-facing counts, as the authority reports them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityGroupReport {
    pub active: usize,
    pub queued: usize,
    pub uncertain: usize,
    pub cooldown_until_ms: u64,
    pub unavailable: bool,
}

/// The authority's state as `status` reads it, tagged with the directory it
/// was read from so every output names the broker it addressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityReport {
    pub directory: PathBuf,
    pub epoch: u64,
    pub journal_healthy: bool,
    pub live_scopes: usize,
    pub groups: BTreeMap<String, AuthorityGroupReport>,
}

/// The outcome of a reset: the new epoch and the directory addressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetReport {
    pub directory: PathBuf,
    pub epoch: u64,
}

/// Why an admin operation could not complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorityAdminError {
    /// No broker is reachable at the addressed directory.
    NotRunning { directory: PathBuf, reason: String },
    /// The broker answered but the operation failed.
    Failed { directory: PathBuf, reason: String },
}

impl std::fmt::Display for AuthorityAdminError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotRunning { directory, reason } => write!(
                f,
                "no admission broker is running for directory {} ({reason})",
                directory.display()
            ),
            Self::Failed { directory, reason } => {
                write!(
                    f,
                    "admission broker at {} failed: {reason}",
                    directory.display()
                )
            }
        }
    }
}

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

/// How a starting process joins the authority (#2024 S3): the pure decision
/// the negotiation use case makes from what the interface observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NegotiationPlan {
    /// No `admission` section and no inherited context: admission is off.
    Disabled,
    /// A top-level session registers a fresh root at `directory`.
    Root { directory: PathBuf },
    /// A descendant binds the capability its parent registered; it inherits
    /// the authority unconditionally and never compares its own config.
    Child { context: PathBuf },
}

/// What the interface observed about a starting process, for the negotiation
/// decision: whether an `admission` section is configured (and where its
/// authority lives), and whether a parent handed down an admission context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NegotiationInputs {
    /// The configured authority directory, when an `admission` section is set.
    pub configured_directory: Option<PathBuf>,
    /// The `--admission-context` sidecar path, when this is a child.
    pub inherited_context: Option<PathBuf>,
}
