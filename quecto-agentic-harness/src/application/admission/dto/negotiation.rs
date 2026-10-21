//! How a starting process joins the authority: what the interface observed
//! and the plan the negotiation use case decides.

use std::path::PathBuf;

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

/// How the authority came to be reachable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityAvailability {
    /// A broker was already answering.
    AlreadyRunning,
    /// This process started one on demand (or lost the start to another
    /// starter) and it now answers.
    StartedOnDemand,
}
