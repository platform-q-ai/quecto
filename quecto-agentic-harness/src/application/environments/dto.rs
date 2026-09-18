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
