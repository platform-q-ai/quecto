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

/// How `/admission restart` brought the broker back (#2615).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartOutcome {
    /// The installed systemd unit serving this config was restarted.
    ServiceRestarted,
    /// The running broker stopped and a fresh one was started on demand.
    Restarted,
    /// No broker was running; one was started on demand.
    Started,
}

/// What `/admission restart` did (#2615).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestartReport {
    pub outcome: RestartOutcome,
    /// The session's link health after rejoining.
    pub health: String,
    /// The broker publishes a different policy than the one this session
    /// composed against, so the session itself must restart to adopt it.
    pub session_restart_required: bool,
}

/// Why `/admission restart` did not complete (#2615).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestartError {
    /// Admission is off in this session.
    NotConfigured,
    /// A child cannot restart the broker its parent's capability lives in.
    Child,
    /// Another restart is running; one at a time.
    InProgress,
    /// The broker could not be restarted.
    Restart(String),
    /// The broker is back, but this session could not rejoin it.
    Rejoin {
        outcome: RestartOutcome,
        reason: String,
    },
}
