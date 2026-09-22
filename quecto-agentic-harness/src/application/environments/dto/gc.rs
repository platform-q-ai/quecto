//! `quecto container gc` at the boundary (#2024 S4d, #2070): the request, its
//! abandoned-run policy, and what the collector reports.

/// What `quecto container gc` is asked to do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GcRequest {
    /// Report without removing anything.
    pub dry_run: bool,
    /// The container config whose scripts list and remove unrecorded
    /// environments (`--name`; `None` is what `container: true` selects:
    /// the repo-bound `standard`, else the labelled default). The
    /// collector scans that config's own state root and the roots the
    /// records it created imply — never another config's directories.
    pub config: Option<String>,
    /// What to do with a directory nothing records whose checkout still
    /// hosts a run its owner never closed (#2070).
    pub abandoned: AbandonedRuns,
}

/// An abandoned run: a swarm whose container is gone or exited, whose
/// directory no registry record names, and whose board still says the run
/// is not over — the master exited before its coordinator and nobody
/// retained the box. Kept by default; collected only when the operator
/// asks, outright or once the directory is old enough.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AbandonedRuns {
    #[default]
    Keep,
    /// `--abandoned`: collect every such directory.
    Collect,
    /// `--abandoned-after <duration>`: collect those at least this old; a
    /// directory whose age cannot be read is never old enough.
    OlderThan { secs: u64 },
}

/// How an orphan would be (or was) removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GcRemoval {
    /// The stopped record's retained `cleanup` argv, which removes the
    /// container and the state dir together; the record is then forgotten.
    RetainedCleanup { environment_ref: String },
    /// No record: the config's `cleanup` argv, given the environment id.
    ConfiguredCleanup { config: String },
    /// A stopped record with nothing left on disk or in the runtime: the
    /// record alone is forgotten.
    ForgetRecord { environment_ref: String },
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

/// The collector's refusal: no container config to list and remove
/// through (the selection's own account, or a config without `inspect`
/// or `cleanup`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcRefused(pub String);

impl std::fmt::Display for GcRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for GcRefused {}

/// The collector's account: what it would remove (dry run) or removed,
/// what it kept, and what went wrong.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GcReport {
    pub dry_run: bool,
    /// The container config whose scripts served the collection.
    pub config: String,
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
