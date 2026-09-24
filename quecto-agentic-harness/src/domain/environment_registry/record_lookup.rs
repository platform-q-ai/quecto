//! The registry's status, target and lookup-error vocabulary: what a record
//! is in, how one is addressed, and why an address fails.

/// Lifecycle status of one committed environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentStatus {
    /// Live and joinable.
    Running,
    /// A kill claim is outstanding; not joinable, not yet stopped.
    Killing,
    /// Kill succeeded; terminal. The record stays listed; its ref is not
    /// reused while it is (#2070). A restore forgets it once nothing of it
    /// is left on disk (#2134).
    Stopped,
    /// Kill failed; retryable via another kill, with `last_error` retained.
    CleanupFailed,
    /// Emptied while its owner had not ended the swarm it hosts (#1924,
    /// #2070) — a lost coordinator, a crash, a run nobody closed: the
    /// final-member kill was deliberately withheld so the board, checkout and
    /// unpushed work survive and the run can be resumed. Killable only by an explicit
    /// `kill_container`; a join is admitted for inspection but never revives
    /// it (no automatic teardown can follow), so a rolled-back or exited
    /// joiner leaves it retained.
    Retained,
}

/// How a caller addresses an existing environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentTarget {
    /// Session-scoped `CN` ref minted by this registry.
    Ref(String),
    /// Optional user-facing environment name; must resolve unambiguously.
    Name(String),
}

/// Resolution failures. Never guesses: unknown, ambiguous, stopped, and stale
/// targets each fail with their own actionable error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentLookupError {
    Unknown(String),
    Ambiguous(String),
    Stopped(String),
    Stale(String),
    /// The durable registry could not be read at startup (round 2 F-B,
    /// #2033): a target this session did not create is not unknown, it is
    /// unknowable, and the store's own account says why.
    Unreadable(String),
}

impl std::fmt::Display for EnvironmentLookupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(t) => write!(f, "environment '{t}' is unknown in this session"),
            Self::Ambiguous(t) => write!(f, "environment name '{t}' is ambiguous in this session"),
            Self::Stopped(t) => write!(f, "environment '{t}' is stopped"),
            Self::Stale(t) => write!(
                f,
                "environment '{t}' is stale: cleanup is pending or failed; retry kill_container"
            ),
            Self::Unreadable(error) => write!(f, "registry unreadable: {error}"),
        }
    }
}

impl std::error::Error for EnvironmentLookupError {}
