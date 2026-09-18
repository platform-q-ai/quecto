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
    /// Whether an environment created from it can be joined later
    /// (`{"mode":"existing"}`): the config carries an `exec` argv.
    pub joinable: bool,
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

// ─── Standard container (#2024 S4e) ─────────────────────────────────────────

/// The name of the container config `quecto container init` writes.
pub const STANDARD_CONTAINER_CONFIG: &str = "standard";

/// The image the standard config launches when `init` is not told
/// another: the tag the official adapter set already defaults to, so the
/// Containerfile init materialises is its missing build input.
pub const STANDARD_CONTAINER_IMAGE: &str = "quecto-box:local";

/// Where the standard bundle lives below a project: the assets are
/// materialised here and the config entry's argv names them here.
pub const STANDARD_CONTAINER_DIR: &str = ".quecto/containers/standard";

/// One file of the embedded standard bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerAsset {
    /// Relative to the bundle directory (`Containerfile`,
    /// `scripts/create.sh`).
    pub path: String,
    pub contents: Vec<u8>,
    pub executable: bool,
}

/// The embedded bundle this binary carries, with its version and the
/// command that builds its image (a template over `{image}` and `{dir}`,
/// the bundle directory): what runtime builds it is the bundle's
/// knowledge, never the application's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerAssetCatalogue {
    pub version: u32,
    pub assets: Vec<ContainerAsset>,
    pub build_command: String,
}

impl ContainerAssetCatalogue {
    /// The build command for `image` from the bundle at `dir`.
    pub fn build_command_for(&self, image: &str, dir: &std::path::Path) -> String {
        self.build_command
            .trim()
            .replace("{image}", image)
            .replace("{dir}", &dir.to_string_lossy())
    }
}

/// What an asset's destination holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetState {
    Missing,
    /// The file exists with exactly the embedded bytes.
    Identical,
    /// The file exists with other bytes (an edit, an older version).
    Differs,
    /// The destination cannot be judged or written through (a symbolic
    /// link, a directory in a file's place); init refuses it.
    Refused,
}

/// What materialising one asset did: an existing file is never replaced
/// unless the run is a refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetOutcome {
    Written,
    KeptIdentical,
    KeptDiffering,
    /// A differing file was replaced with the embedded bytes (`--refresh`).
    Refreshed,
}

/// `quecto container init` as the use case receives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandardContainerRequest {
    /// The project (checkout) the bundle and overlay belong to; absolute.
    pub project: std::path::PathBuf,
    /// `--repo <url>`; `None` derives the checkout's `origin` remote.
    pub repository: Option<String>,
    /// `--image <tag>`; `None` is [`STANDARD_CONTAINER_IMAGE`].
    pub image: Option<String>,
    /// Report what would be written; write nothing.
    pub dry_run: bool,
    /// `--refresh`: replace a materialised asset whose bytes differ from
    /// the embedded ones (an edit, an older bundle) instead of keeping it.
    pub refresh: bool,
}

/// Where the repository the entry bakes in came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepositoryOrigin {
    /// `--repo` on the command line.
    Explicit,
    /// The existing standard entry's own `--repo` (a re-init without the
    /// flag keeps it; the origin is not re-derived).
    ExistingEntry,
    /// The checkout's `origin` remote.
    CheckoutOrigin,
    /// Neither: the entry is a sandbox (empty workspace, no clone).
    Sandbox,
}

/// On a re-init, how one of the entry's values (`--repo`, `--image`)
/// relates to the existing standard entry's: kept as it was, or rewritten
/// by the flag (`previous` is the old value; `None` for a sandbox that
/// gains a repository).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryValueChange {
    Kept,
    Rewrote { previous: Option<String> },
}

/// A `container_configs.<name>` entry as init writes it: every argv
/// names the materialised script by absolute path; `default` is the
/// `"default": true` label. Composition maps it onto the configuration
/// document; the use case never handles JSON.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContainerConfigDocument {
    pub default: bool,
    pub create: Vec<String>,
    pub exec: Vec<String>,
    pub inspect: Vec<String>,
    pub kill: Vec<String>,
    pub cleanup: Vec<String>,
}

impl ContainerConfigDocument {
    /// The value after `flag` in the create argv, before any `--`
    /// (the shipped scripts' convention: `--repo <url>`, `--image <tag>`).
    pub fn create_value(&self, flag: &str) -> Option<&str> {
        self.create
            .iter()
            .take_while(|arg| *arg != "--")
            .skip_while(|arg| *arg != flag)
            .nth(1)
            .map(String::as_str)
    }

    /// Every argv with its key, in the order the entry lists them.
    pub fn argvs(&self) -> [(&'static str, &[String]); 5] {
        [
            ("create", &self.create),
            ("exec", &self.exec),
            ("inspect", &self.inspect),
            ("kill", &self.kill),
            ("cleanup", &self.cleanup),
        ]
    }
}

/// The config entry init wrote (or would write).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandardEntryOutcome {
    pub name: String,
    /// The overlay file the entry lives in; `None` on a dry run without
    /// a known location.
    pub path: Option<std::path::PathBuf>,
    /// Whether the entry carries `"default": true`.
    pub default: bool,
    /// The name of the entry that is already the default, when ours is
    /// not.
    pub existing_default: Option<String>,
    /// The entry as written, so the presenter can show it verbatim.
    pub entry: ContainerConfigDocument,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandardContainerReport {
    pub assets_dir: std::path::PathBuf,
    pub version: u32,
    pub written: Vec<std::path::PathBuf>,
    pub kept: Vec<std::path::PathBuf>,
    /// Existing files that differ from the embedded bytes; kept as they
    /// are (`--refresh` replaces them).
    pub differing: Vec<std::path::PathBuf>,
    /// Differing files replaced with the embedded bytes (`--refresh`).
    pub refreshed: Vec<std::path::PathBuf>,
    pub repository: Option<String>,
    pub repository_origin: RepositoryOrigin,
    pub image: String,
    /// How `--repo` and `--image` relate to an existing standard entry's
    /// values; `None` when there was no entry to keep from.
    pub repository_change: Option<EntryValueChange>,
    pub image_change: Option<EntryValueChange>,
    /// The exact command that builds `image` from the materialised
    /// bundle: the one step init leaves to the operator.
    pub build_command: String,
    pub entry: StandardEntryOutcome,
    pub dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitialiseStandardContainerError {
    /// The project path is not absolute.
    ProjectNotAbsolute(std::path::PathBuf),
    /// The quecto base directory (where the state dir goes) is not
    /// absolute; the argv written must not depend on a later cwd.
    BaseDirNotAbsolute(std::path::PathBuf),
    /// The repository URL (`--repo`, or the checkout's `origin`) embeds a
    /// credential — any userinfo over http(s) (`token@host`,
    /// `user:password@host`), a password over the other schemes; init
    /// refuses to bake it into the shareable overlay. The URL is carried
    /// redacted.
    RepositoryCarriesCredentials {
        url: String,
        origin: RepositoryOrigin,
    },
    /// An asset could not be observed or written. `entry_written` says
    /// the overlay entry already landed before the failure, so the
    /// message tells the operator how to finish or roll back.
    Asset {
        path: std::path::PathBuf,
        reason: String,
        entry_written: bool,
    },
    /// The checkout's origin (or its toplevel) could not be read.
    Origin(String),
    /// The project lies inside a checkout but is not its root: the
    /// overlay written there is one an agent started at the root never
    /// reads. Carries the root to pass instead.
    NotRepositoryRoot {
        project: std::path::PathBuf,
        toplevel: std::path::PathBuf,
    },
    /// The effective container-config set could not be read, or the
    /// checkout's overlay is withheld (untrusted): init never trusts an
    /// overlay it did not write.
    Configuration(String),
    /// The overlay entry could not be written (the configuration
    /// capability's own account, e.g. an untrusted overlay).
    Persist(String),
}

impl std::fmt::Display for InitialiseStandardContainerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ProjectNotAbsolute(path) => {
                write!(f, "--project must be an absolute path: {}", path.display())
            }
            Self::BaseDirNotAbsolute(path) => write!(
                f,
                "the quecto base directory must be an absolute path: {}",
                path.display()
            ),
            Self::RepositoryCarriesCredentials { url, origin } => write!(
                f,
                "{} {url} carries a credential in its userinfo (any `user@` or `user:password@` over http(s) is a token or password); init will not bake it into the repo-local overlay — use a URL without any credential (a credential helper, an ssh key or `gh auth login` supplies it at clone time)",
                match origin {
                    RepositoryOrigin::Explicit => "the --repo URL",
                    RepositoryOrigin::ExistingEntry => "the existing standard entry's --repo",
                    _ => "the checkout's origin remote",
                }
            ),
            Self::Asset {
                path,
                reason,
                entry_written: false,
            } => write!(f, "asset {}: {reason}", path.display()),
            Self::Asset {
                path,
                reason,
                entry_written: true,
            } => write!(
                f,
                "asset {}: {reason}; the overlay entry container_configs.standard was already written — fix the path and run init again, or roll back with `quecto config unset --local container_configs.standard`",
                path.display()
            ),
            Self::Origin(reason) => write!(f, "cannot read the checkout's origin: {reason}"),
            Self::NotRepositoryRoot { project, toplevel } => write!(
                f,
                "{} is not the repository root ({}): the overlay written here would be one an agent started at the root never reads — run init from the root, or pass --project {}",
                project.display(),
                toplevel.display(),
                toplevel.display()
            ),
            Self::Configuration(reason) => write!(f, "{reason}"),
            Self::Persist(reason) => write!(f, "{reason}"),
        }
    }
}

impl std::error::Error for InitialiseStandardContainerError {}

/// `quecto container status`: where the standard bundle stands for a
/// project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandardContainerStatus {
    pub assets_dir: std::path::PathBuf,
    pub version: u32,
    pub assets: Vec<(std::path::PathBuf, AssetState)>,
    /// The `standard` entry of the effective set, when present.
    pub entry: Option<ContainerConfigEntry>,
    /// The checkout's overlay was not applied (untrusted or refused).
    pub overlay_withheld: bool,
    pub diagnostics: Vec<String>,
    /// The create preflight's `image` check for the entry, when the
    /// preflight could run.
    pub image: Option<PreflightCheck>,
    /// Why the preflight could not run (no entry, a script that refuses
    /// the mode), when it could not.
    pub preflight_error: Option<String>,
}

impl StandardContainerStatus {
    /// Assets that are in place as embedded or as edited.
    pub fn assets_present(&self) -> usize {
        self.assets
            .iter()
            .filter(|(_, state)| matches!(state, AssetState::Identical | AssetState::Differs))
            .count()
    }

    pub fn assets_differing(&self) -> usize {
        self.assets
            .iter()
            .filter(|(_, state)| *state == AssetState::Differs)
            .count()
    }

    /// Everything a container spawn needs is in place: every asset as
    /// embedded, the entry present, the overlay applied, the image
    /// present.
    pub fn healthy(&self) -> bool {
        self.assets_present() == self.assets.len()
            && self.assets_differing() == 0
            && self.entry.is_some()
            && !self.overlay_withheld
            && self
                .image
                .as_ref()
                .is_some_and(|check| check.status == CheckStatus::Passed)
    }
}

/// How a persisted container-config entry landed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedContainerConfig {
    pub path: std::path::PathBuf,
    pub created: bool,
}
