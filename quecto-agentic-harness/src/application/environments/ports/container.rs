//! Container configs, assets and the container runtime: lookup, preflight,
//! inventory, the config roster, the asset store, the workspace origin and
//! config persistence.

use crate::application::environments::dto::AssetOutcome;
use crate::application::environments::dto::AssetState;
use crate::application::environments::dto::ContainerAsset;
use crate::application::environments::dto::ContainerAssetCatalogue;
use crate::application::environments::dto::ContainerConfigDocument;
use crate::application::environments::dto::ContainerConfigEntry;
use crate::application::environments::dto::ContainerRuntimeTarget;
use crate::application::environments::dto::DiagnosableContainerConfig;
use crate::application::environments::dto::EnvironmentLiveness;
use crate::application::environments::dto::EnvironmentStateDir;
use crate::application::environments::dto::PersistedContainerConfig;
use crate::application::environments::dto::PreflightCheck;
use crate::application::environments::dto::RuntimeContainer;
use std::path::Path;
use std::path::PathBuf;

// ─── Container-runtime diagnosis (#2024 S4b) ─────────────────────────────────

/// The container config a diagnosis targets, resolved the way a launch
/// resolves it: the effective configuration of the working directory
/// (its trusted overlay merged over the global file) or an explicit file,
/// the named entry or what `container: true` selects (the repo-bound
/// `standard`, else the labelled default). Implemented by infrastructure
/// over the launch policy's selection; composition binds the checkout.
pub trait ContainerConfigLookup: Send + Sync {
    fn lookup(&self, target: &ContainerRuntimeTarget)
    -> Result<DiagnosableContainerConfig, String>;
}

/// The create script's own preflight, run without creating an
/// environment: which binaries exist, whether the image is present,
/// whether the repository is reachable, whether the state dir is
/// writable. One list of checks serves the create and the doctor, so the
/// adapter asks the script rather than reimplementing it. `Err` carries
/// why no checks could be obtained (the script refuses the mode, is
/// missing, or reported nothing).
pub trait ContainerRuntimePreflight: Send + Sync {
    fn preflight(&self, config: &DiagnosableContainerConfig)
    -> Result<Vec<PreflightCheck>, String>;
}

/// The host's inventory of environments outside any registry, through a
/// container config's own scripts (the harness knows no runtime): every
/// environment the runtime knows (`inspect --list`), the state
/// directories under a state root, and the removal of one environment by
/// id (the config's `cleanup`, which takes the container and the state
/// dir down together).
pub trait ContainerRuntimeInventory: Send + Sync {
    fn containers(
        &self,
        config: &DiagnosableContainerConfig,
    ) -> Result<Vec<RuntimeContainer>, String>;
    /// One environment's liveness through the config's `inspect` argv
    /// given its id — the authority for a state directory, whatever the
    /// listing labelled.
    fn inspect(
        &self,
        config: &DiagnosableContainerConfig,
        environment_id: &str,
    ) -> EnvironmentLiveness;
    fn environment_dirs(&self, root: &Path) -> Result<Vec<EnvironmentStateDir>, String>;
    /// The host's canonical form of a state root (symlinks resolved), so
    /// two spellings of one directory compare equal (round 3 M1, #2033);
    /// a root the host cannot resolve is its own canonical form.
    fn canonical_root(&self, root: &Path) -> std::path::PathBuf;
    fn remove(
        &self,
        config: &DiagnosableContainerConfig,
        environment_id: &str,
    ) -> Result<(), String>;
}

// ─── Container-config discovery (#2024 S4c) ─────────────────────────────────

/// The raw effective container-config set of the launching agent, read the
/// way a launch reads it: the global file with the checkout's trusted
/// overlay merged in, each entry marked with the layer that declared it,
/// plus whether an overlay was withheld and the layer diagnostics. The
/// adapter reports the set as configured; the listing use case owns the
/// rules over it (default visibility, ordering). `Err` carries why no set
/// could be read (no configuration composed, an invalid file).
pub trait ContainerConfigRoster: Send + Sync {
    fn roster(&self) -> Result<ContainerConfigRosterReport, String>;

    /// An opaque token that changes whenever the roster could have
    /// changed (a configuration layer or the trust record was written)
    /// and is otherwise stable — cheap enough to ask for on every render,
    /// so a presenter can cache the rendered roster against it instead of
    /// reading the configuration each time.
    fn revision(&self) -> String;
}

/// What the roster port reports before the listing rules are applied.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContainerConfigRosterReport {
    pub configs: Vec<ContainerConfigEntry>,
    pub overlay_withheld: bool,
    pub diagnostics: Vec<String>,
}

// ─── Standard container (#2024 S4e) ─────────────────────────────────────────

/// The embedded standard bundle (Containerfile, runtime scripts) and its
/// materialisation below a project. The catalogue is what this binary
/// carries; `observe` compares a destination with the embedded bytes;
/// `materialise` writes a missing asset (with its mode) and never
/// replaces an existing file, so an operator's edit survives a re-init.
/// The use case owns where the bundle goes and what a difference means.
pub trait ContainerAssetStore: Send + Sync {
    fn catalogue(&self) -> ContainerAssetCatalogue;

    /// What the destination of `asset` below `dir` holds; `Err` when it
    /// cannot be judged or written through (a symbolic link in the
    /// file's place or in any directory between `root` and it), so a
    /// dry run and a status see exactly what a materialise would refuse.
    fn observe(
        &self,
        root: &Path,
        dir: &Path,
        asset: &ContainerAsset,
    ) -> Result<AssetState, String>;

    /// Write `asset` below `dir`, which lies below `root` (the project):
    /// no directory between `root` (exclusive) and the asset may be a
    /// symbolic link, so a swapped directory cannot redirect the write
    /// outside the project.
    fn materialise(
        &self,
        root: &Path,
        dir: &Path,
        asset: &ContainerAsset,
    ) -> Result<AssetOutcome, String>;

    /// As `materialise`, but a regular file holding other bytes is
    /// replaced whole (temporary file, rename) with the embedded ones and
    /// its mode: `init --refresh`, the way back from an edit or an older
    /// bundle. The same symbolic-link refusals apply.
    fn refresh(
        &self,
        root: &Path,
        dir: &Path,
        asset: &ContainerAsset,
    ) -> Result<AssetOutcome, String>;

    /// Replace `asset` below `dir` only while it still holds exactly bytes
    /// an earlier quecto shipped (#2206): the bytes are re-read and
    /// re-judged at replacement time, and the file must be unchanged up to
    /// the rename. Anything else — an edit made since it was judged
    /// outdated, a file gone — is refused (`Err`) and left as it is, so
    /// no one's edit is ever overwritten by a refresh nobody asked for.
    fn refresh_outdated(
        &self,
        root: &Path,
        dir: &Path,
        asset: &ContainerAsset,
    ) -> Result<AssetOutcome, String>;
}

/// The repository a checkout came from: its `origin` remote URL, `None`
/// when the directory is not a git checkout or has no `origin`. `Err`
/// carries why it could not be asked (git missing).
pub trait WorkspaceOrigin: Send + Sync {
    fn origin(&self, checkout: &Path) -> Result<Option<String>, String>;

    /// The root of the working tree `checkout` lies in (`git rev-parse
    /// --show-toplevel`), `None` when it is not inside a checkout.
    /// `Err` carries why it could not be asked (git missing).
    fn toplevel(&self, checkout: &Path) -> Result<Option<PathBuf>, String>;
}

/// Records one `container_configs.<name>` entry in the project's
/// repo-local overlay through the configuration capability's one safe
/// write path (composition maps this port onto it): validated as a layer
/// and as the merge, trust recorded for exactly the bytes written, an
/// untrusted overlay refused in that capability's own words.
pub trait ContainerConfigPersistence: Send + Sync {
    /// Whether a persist would be accepted as things stand (an overlay
    /// location exists, the overlay's current content is trusted or
    /// absent): the refusal a persist would give, before anything else
    /// is written. Nothing is written.
    fn check(&self) -> Result<(), String>;

    /// Write `container_configs.<name>` as `entry`; when
    /// `displace_default` names another overlay entry, its `"default":
    /// true` label is removed in the same write (#2035: the merge accepts
    /// one default, so the two changes must land together). Naming an
    /// entry the overlay does not declare is an error, nothing written.
    fn persist(
        &self,
        name: &str,
        entry: &ContainerConfigDocument,
        displace_default: Option<&str>,
    ) -> Result<PersistedContainerConfig, String>;

    /// The overlay file a persist would write, when the run has one.
    fn location(&self) -> Option<PathBuf>;

    /// The entry `container_configs.<name>` as the project's own overlay
    /// currently declares it (`None` when the overlay has none), so a
    /// re-init can keep what it wrote before. Read from the applied
    /// overlay only — never a global entry of the same name; `Err` when
    /// the overlay cannot be read.
    fn existing(&self, name: &str) -> Result<Option<ContainerConfigDocument>, String>;
}
