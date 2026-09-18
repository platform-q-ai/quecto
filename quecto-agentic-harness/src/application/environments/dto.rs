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

// ─── Container-config discovery (#2024 S4c) ─────────────────────────────────

/// Which configuration layer declared a container config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerConfigLayer {
    /// The launching agent's checkout, through its applied
    /// `.quecto/config.json` overlay (repo-bound).
    Overlay,
    /// The global file (or the explicit `--config` file).
    Global,
}

impl ContainerConfigLayer {
    /// The wire word agents read (`overlay`, `global`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Overlay => "overlay",
            Self::Global => "global",
        }
    }
}

/// One container config as the inventory presents it: what an agent needs
/// to choose one, never the argv (which is operator territory).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerConfigEntry {
    pub name: String,
    /// What `container: true` selects — exactly as launch policy would:
    /// false on every entry while the checkout's overlay is withheld
    /// (the default is then unknown and an implicit selection refused),
    /// on an entry a launch would refuse, and on all of them when more
    /// than one is labelled default.
    pub default: bool,
    pub layer: ContainerConfigLayer,
    /// The repository a new environment clones, when the config bakes
    /// one in; `None` for a sandbox (empty workspace).
    pub repository: Option<String>,
    /// Why a launch would refuse this entry as configured (a missing or
    /// unsafe argv); `None` when it is launchable.
    pub problem: Option<String>,
}

/// The container configs in effect for the launching agent (#2024 S4c),
/// as the inventory reports them: the `container: true` default first,
/// then the rest by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerConfigInventory {
    pub configs: Vec<ContainerConfigEntry>,
    /// The checkout has an overlay that was NOT applied and could have
    /// changed the set: `configs` is the global set alone, no entry is
    /// the default, and `diagnostics` says how to trust the overlay.
    pub overlay_withheld: bool,
    /// The configuration layer diagnostics (an untrusted or refused
    /// overlay, a retired local file), one line each.
    pub diagnostics: Vec<String>,
}

impl ContainerConfigInventory {
    /// The entry `container: true` selects, if any.
    pub fn default_entry(&self) -> Option<&ContainerConfigEntry> {
        self.configs.iter().find(|entry| entry.default)
    }
}
