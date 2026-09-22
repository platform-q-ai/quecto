//! Container config selection at launch (#2024 S4a): the boundary models
//! of `SelectContainerConfig`.

use std::fmt;

use super::super::standard_script::StandardScriptVerdict;

/// Where the container configs a launch selects from are read: the
/// launching agent's own effective configuration (its base file with its
/// checkout's trusted overlay merged in) or one explicit file the spawn
/// call named, which replaces the layers as `--config` does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerConfigSource {
    LaunchingAgent,
    Explicit(std::path::PathBuf),
}

/// One named container config as launch policy sees it: the argv sets a
/// script-managed runtime runs, whether `container: true` selects it, and
/// what an agent choosing between entries needs to know (#2024 S4c):
/// whether the checkout's applied overlay declared it, and the repository
/// its create argv bakes in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerLaunchConfig {
    pub name: String,
    pub default: bool,
    pub create: Vec<String>,
    pub cleanup: Vec<String>,
    pub exec: Vec<String>,
    pub kill: Vec<String>,
    pub inspect: Vec<String>,
    /// Declared by the launching agent's checkout through its applied
    /// `.quecto/config.json` overlay (repo-bound), not by the global file.
    pub repo_bound: bool,
    /// The repository the create argv bakes in (`--repo <url>` for the
    /// shipped scripts); `None` for a sandbox config or an adapter whose
    /// argv names none in that form.
    pub repository: Option<String>,
}

impl ContainerLaunchConfig {
    /// The programs the argv sets run (each set's first element), once
    /// each in `create`, `cleanup`, `exec`, `kill`, `inspect` order: the
    /// host-side scripts a launch executes.
    pub fn scripts(&self) -> Vec<std::path::PathBuf> {
        let mut scripts: Vec<std::path::PathBuf> = Vec::new();
        for argv in [
            &self.create,
            &self.cleanup,
            &self.exec,
            &self.kill,
            &self.inspect,
        ] {
            if let Some(first) = argv.first().map(std::path::PathBuf::from)
                && !scripts.contains(&first)
            {
                scripts.push(first);
            }
        }
        scripts
    }

    /// Why launch policy would refuse this entry's argv, if it would:
    /// `create` and `cleanup` are required, and no argument of any set may
    /// be empty or carry a NUL. One rule for the selection and the roster.
    pub fn argv_problem(&self) -> Option<&'static str> {
        if self.create.is_empty() {
            return Some("missing create argv");
        }
        if self.cleanup.is_empty() {
            return Some("missing cleanup argv");
        }
        let unsafe_arg = |arg: &String| arg.is_empty() || arg.contains('\0');
        self.create
            .iter()
            .chain(&self.cleanup)
            .chain(&self.exec)
            .chain(&self.kill)
            .chain(&self.inspect)
            .any(unsafe_arg)
            .then_some("unsafe argv")
    }
}

/// The container configs in effect for a source, sorted by name, and the
/// layer diagnostics the configuration capability reported while
/// resolving them (an untrusted or refused overlay that was not applied,
/// a retired local file that is no longer loaded).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EffectiveContainerConfigSet {
    pub configs: Vec<ContainerLaunchConfig>,
    pub diagnostics: Vec<String>,
    /// The checkout has an overlay that was NOT applied and could have
    /// changed the default (refused, unparseable, or declaring
    /// `container_configs`): `configs` is the global set alone, and
    /// whatever default the overlay labels is unknown to launch policy.
    /// An unapplied overlay that cannot have touched the container set
    /// leaves this false and travels in `diagnostics` alone.
    pub overlay_withheld: bool,
}

/// The name of the entry `quecto container init` writes into a repo's
/// overlay (#2035): when the launching agent's checkout declares it, it
/// is that repo's default — `container: true` selects it whatever entry
/// the global file or another overlay entry labels `"default": true`.
/// The environments capability spells the same name in its own constant;
/// the contract suite holds the two equal.
pub const REPO_STANDARD_CONTAINER: &str = "standard";

impl EffectiveContainerConfigSet {
    /// The configured names, sorted.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.configs.iter().map(|c| c.name.clone()).collect();
        names.sort_unstable();
        names
    }

    /// The checkout's own `standard` entry (#2035): declared by its
    /// applied overlay, never a global entry of that name.
    pub fn repo_standard(&self) -> Option<&ContainerLaunchConfig> {
        self.configs
            .iter()
            .find(|config| config.repo_bound && config.name == REPO_STANDARD_CONTAINER)
    }
}

/// Why the effective container configs of a source could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerConfigsError {
    /// No launching-agent configuration was composed for this launcher and
    /// the spawn call named no file.
    NoSource,
    /// The configuration could not be loaded or is invalid; the reason is
    /// the configuration capability's own diagnostic, naming the file(s).
    Invalid(String),
}

impl fmt::Display for ContainerConfigsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSource => {
                f.write_str("container spawn requires --config so container_configs can be loaded")
            }
            Self::Invalid(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for ContainerConfigsError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectContainerConfigRequest {
    pub source: ContainerConfigSource,
    /// `container_config: "<name>"`; `None` selects the checkout's
    /// repo-bound `standard` entry, else the labelled default.
    pub name: Option<String>,
}

/// The config a new container launches with, and what the launcher should
/// tell the operator about the layers it was read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedContainerConfig {
    pub config: ContainerLaunchConfig,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectContainerConfigError {
    /// The spawn call named a relative file: only an absolute, trusted
    /// path may supply argv that this process will execute.
    RelativeConfigPath(std::path::PathBuf),
    Unavailable(ContainerConfigsError),
    /// `container: true` from a checkout whose overlay was not applied and
    /// could have changed the default (#2024 S4a: refused, unparseable,
    /// or declaring `container_configs`): the default it labels is
    /// unknown, so an implicit selection must not quietly land in the
    /// global one. The layer diagnostics say why the overlay was withheld.
    OverlayWithheld {
        diagnostics: Vec<String>,
    },
    /// `container: true` with no entry labelled `"default": true`. The
    /// layer diagnostics ride along: a withheld overlay is the likely
    /// reason the expected entry is missing.
    NoDefault {
        available: Vec<String>,
        diagnostics: Vec<String>,
    },
    Unknown {
        name: String,
        available: Vec<String>,
        diagnostics: Vec<String>,
    },
    /// The selected entry cannot be run: `what` names the argv fault.
    InvalidArgv {
        name: String,
        what: &'static str,
    },
    /// A standard-bundle script the entry's argv names no longer carries
    /// the embedded bytes (or is missing, or cannot be judged): the
    /// host-side script would run with no trust behind it, so the launch
    /// is refused until `quecto container init --refresh` restores it.
    StandardScriptAltered {
        name: String,
        script: std::path::PathBuf,
        verdict: StandardScriptVerdict,
    },
}

fn available(names: &[String]) -> String {
    if names.is_empty() {
        "none configured".to_string()
    } else {
        names.join(", ")
    }
}

/// The layer diagnostics appended to an error line, one
/// `; Configuration diagnostics: <line>` each, so the reason an entry is
/// missing (a withheld overlay) is never dropped from the text.
fn appended(diagnostics: &[String]) -> String {
    diagnostics
        .iter()
        .map(|line| format!("; Configuration diagnostics: {line}"))
        .collect()
}

impl SelectContainerConfigError {
    /// The configuration-layer diagnostics the failed selection carried,
    /// for a caller that reports them on its own channel too.
    pub fn diagnostics(&self) -> &[String] {
        match self {
            Self::OverlayWithheld { diagnostics }
            | Self::NoDefault { diagnostics, .. }
            | Self::Unknown { diagnostics, .. } => diagnostics,
            Self::RelativeConfigPath(_)
            | Self::Unavailable(_)
            | Self::InvalidArgv { .. }
            | Self::StandardScriptAltered { .. } => &[],
        }
    }
}

impl fmt::Display for SelectContainerConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RelativeConfigPath(_) => {
                f.write_str("container spawn requires an absolute trusted config path")
            }
            Self::Unavailable(error) => write!(f, "{error}"),
            Self::OverlayWithheld { diagnostics } => write!(
                f,
                "container: true refused: the checkout's repo-local config overlay was not applied, so the container config it labels default is unknown ({}); trust it, or name a container_config explicitly to launch from the global configuration",
                diagnostics.join("; ")
            ),
            Self::NoDefault {
                available: names,
                diagnostics,
            } => write!(
                f,
                "no container config is labeled \"default\": true (available container configs: {}){}",
                available(names),
                appended(diagnostics)
            ),
            Self::Unknown {
                name,
                available: names,
                diagnostics,
            } => write!(
                f,
                "unknown container config '{name}' (available container configs: {}){}",
                available(names),
                appended(diagnostics)
            ),
            Self::InvalidArgv { what, .. } => {
                write!(f, "invalid container_configs configuration: {what}")
            }
            Self::StandardScriptAltered {
                name,
                script,
                verdict,
            } => {
                let reason = verdict.refusal(script).unwrap_or_else(|| {
                    format!("{} is not the standard bundle's", script.display())
                });
                write!(f, "container config '{name}' refused: {reason}")
            }
        }
    }
}

impl std::error::Error for SelectContainerConfigError {}
