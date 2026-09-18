//! Records of the environments capability's container-runtime diagnosis
//! (#2024 S4b): what the doctor is asked about, what the preflight found,
//! and the diagnosis the interface presents. Values only; the use case
//! owns the rules over them.

/// Which container config to diagnose, within the run's own configuration
/// selection (the working directory's effective layers, or the explicit
/// `--config` file composition bound).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContainerRuntimeTarget {
    /// `--name <config>`; `None` selects the labelled default.
    pub name: Option<String>,
}

/// The config a diagnosis runs against: its name, the `create` argv whose
/// script hosts the preflight, and the layer diagnostics the selection
/// reported while resolving it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosableContainerConfig {
    pub name: String,
    pub create: Vec<String>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckStatus {
    Passed,
    /// A shortcoming a create survives (no `gh`: members get no token).
    Warned,
    Failed,
}

impl CheckStatus {
    /// The wire word a script prints (`ok`, `warn`, `fail`).
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "ok" => Some(Self::Passed),
            "warn" => Some(Self::Warned),
            "fail" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// One preflight check: what was checked, what was found, and — for a
/// failed or warned check — what to do about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightCheck {
    pub name: String,
    pub status: CheckStatus,
    pub detail: String,
    pub remedy: String,
}

/// The doctor's finding for one container config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerRuntimeDiagnosis {
    pub config: String,
    pub create: Vec<String>,
    pub checks: Vec<PreflightCheck>,
    /// Layer diagnostics the selection reported (an untrusted overlay).
    pub diagnostics: Vec<String>,
}

impl ContainerRuntimeDiagnosis {
    /// True when no check failed; warnings do not count against it.
    pub fn healthy(&self) -> bool {
        self.failed() == 0
    }

    pub fn failed(&self) -> usize {
        self.checks
            .iter()
            .filter(|check| check.status == CheckStatus::Failed)
            .count()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnoseContainerRuntimeError {
    /// The target could not be resolved to a container config (unknown
    /// name, no default, unreadable layers): the selection's own account.
    ConfigUnavailable(String),
    /// The config's create script could not run the preflight: it is
    /// missing, refuses `--preflight-only`, or reported no checks.
    PreflightUnavailable { config: String, detail: String },
}

impl std::fmt::Display for DiagnoseContainerRuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConfigUnavailable(detail) => write!(f, "{detail}"),
            Self::PreflightUnavailable { config, detail } => {
                write!(f, "container config '{config}': {detail}")
            }
        }
    }
}

impl std::error::Error for DiagnoseContainerRuntimeError {}

// ─── Durable environments (#2024 S4d) ────────────────────────────────────────

/// What the runtime says about an environment's container when a restored
/// record is checked against reality.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentLiveness {
    /// The container is up.
    Running,
    /// The container is gone or exited: the record is stale.
    Gone,
    /// The runtime could not be asked (no retained inspect, the script
    /// failed or timed out): the record is kept as it was, with the reason.
    Unknown(String),
}

/// What restoring the durable registry into a session found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RestoredRegistry {
    /// Refs restored as they were recorded, still live.
    pub restored: Vec<String>,
    /// Refs whose container was gone and that were marked stopped.
    pub stopped: Vec<String>,
    /// Refs kept as recorded because the runtime could not be asked, each
    /// with the reason.
    pub unverified: Vec<(String, String)>,
    /// Why nothing (or not everything) could be restored: the store's own
    /// account. Empty when the store read cleanly.
    pub diagnostics: Vec<String>,
}

/// One container the runtime knows under the `quecto.environment_id`
/// label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeContainer {
    pub name: String,
    /// The label's value, when the runtime reported one.
    pub environment_id: Option<String>,
    pub running: bool,
}

/// One environment state directory under a state root, as the shipped
/// scripts lay it out: `<root>/<environment_id>/` with a `container` file
/// naming the runtime container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentStateDir {
    pub path: std::path::PathBuf,
    pub environment_id: String,
    pub container: Option<String>,
}

/// What `quecto container gc` is asked to do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GcRequest {
    /// Report without removing anything.
    pub dry_run: bool,
    /// State roots to scan besides those the registry's records imply.
    pub state_roots: Vec<std::path::PathBuf>,
}

/// How an orphan would be (or was) removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GcRemoval {
    /// The stopped record's retained `cleanup` argv, which removes the
    /// container and the state dir together.
    RetainedCleanup { environment_ref: String },
    /// No record: the runtime container (if any) is removed, then the
    /// state dir (if any).
    Direct,
}

/// One environment the collector judged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcCandidate {
    pub environment_id: String,
    pub state_dir: Option<std::path::PathBuf>,
    pub container: Option<String>,
    pub removal: GcRemoval,
    /// Why it is an orphan.
    pub reason: String,
}

/// One environment the collector left alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcKept {
    pub environment_id: String,
    pub reason: String,
}

/// The collector's account: what it would remove (dry run) or removed,
/// what it kept, and what went wrong.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GcReport {
    pub dry_run: bool,
    /// The state roots that were scanned.
    pub state_roots: Vec<std::path::PathBuf>,
    pub removable: Vec<GcCandidate>,
    /// Candidates whose removal was attempted and reported success (empty
    /// on a dry run).
    pub removed: Vec<GcCandidate>,
    pub kept: Vec<GcKept>,
    /// Removal or inventory failures, each naming what failed.
    pub errors: Vec<String>,
}
