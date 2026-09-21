//! The standard bundle's asset vocabulary: one file of the bundle, whose it
//! is once it exists in a project (#2073), what its destination holds and
//! what materialising it did.

/// Whose file an asset is once it exists in a project (#2073).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetOwnership {
    /// The bundle's: a trusted runtime script. Other bytes are drift, a
    /// launch refuses them and `--refresh` restores them.
    Bundle,
    /// The project's: the Containerfile. The bundle only supplies a starter
    /// when none exists; the project's version is never drift and is never
    /// replaced.
    Project,
}

/// One file of the embedded standard bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerAsset {
    /// Relative to the bundle directory (`Containerfile`,
    /// `scripts/create.sh`).
    pub path: String,
    pub contents: Vec<u8>,
    pub executable: bool,
    pub ownership: AssetOwnership,
}

impl ContainerAsset {
    /// The destination holds the project's own version of a project-owned
    /// file: other bytes there are not drift.
    pub fn is_projects_own(&self, observed: AssetState) -> bool {
        self.ownership == AssetOwnership::Project && observed == AssetState::Differs
    }

    /// The destination holds other bytes than the bundle's in a file the
    /// bundle owns: drift.
    pub fn is_drift(&self, observed: AssetState) -> bool {
        self.ownership == AssetOwnership::Bundle && observed == AssetState::Differs
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
    /// The project's own version of a project-owned file was left alone.
    KeptOwn,
    /// A differing file was replaced with the embedded bytes (`--refresh`).
    Refreshed,
}
