//! How long a UDS harness lives (#1937, epic #1929).
//!
//! Decided once at startup from two facts and never revisited:
//!
//! - a **top-level** harness (started by a user or an external client) exits
//!   when its last client disconnects, unless the user asked for `--persist`,
//!   in which case only an explicit shutdown ends it;
//! - a **launcher-created** child (started with a parent control credential,
//!   #1935) is lifetime-scoped to its launcher: ordinary client churn never
//!   ends it — readiness probes, inspectors and tools come and go — and the
//!   loss of the launch-bound parent control connection (or the bind
//!   deadline) runs the common shutdown. Such a child is never persistent:
//!   it cannot outlive its launcher, so `--persist` is refused at startup
//!   rather than silently narrowed.
//!
//! Pure policy: no flags parsing, no sockets, no processes.
use std::fmt;

/// The lifetime a harness runs with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessLifetime {
    /// Top-level default: exit once the last client has disconnected.
    UntilLastClientDisconnects,
    /// Top-level `--persist`: stay alive across client churn; only an
    /// explicit shutdown (signal or protocol) ends the harness.
    Persistent,
    /// Launcher-created child: bound to its launcher's control connection.
    /// Client churn is ignored; parent loss or the bind deadline ends it.
    LaunchBound,
}

/// Why a requested lifetime is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessLifetimeError {
    /// `--persist` was requested for a launcher-created child.
    LaunchedChildCannotPersist,
}

impl fmt::Display for HarnessLifetimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LaunchedChildCannotPersist => f.write_str(
                "a launcher-created child is lifetime-bound to its launcher and cannot persist; --persist is refused with --parent-control",
            ),
        }
    }
}

impl std::error::Error for HarnessLifetimeError {}

impl HarnessLifetime {
    /// Resolve the lifetime from the startup facts: whether `--persist` was
    /// requested and whether the harness was launched with a parent control
    /// credential. The only refused combination is a persistent launched
    /// child.
    pub fn resolve(persist_requested: bool, launched: bool) -> Result<Self, HarnessLifetimeError> {
        match (launched, persist_requested) {
            (true, false) => Ok(Self::LaunchBound),
            (true, true) => Err(HarnessLifetimeError::LaunchedChildCannotPersist),
            (false, true) => Ok(Self::Persistent),
            (false, false) => Ok(Self::UntilLastClientDisconnects),
        }
    }

    /// Affirmative: only the top-level default lifetime ends the harness
    /// when its last client disconnects.
    pub fn exits_when_last_client_disconnects(self) -> bool {
        matches!(self, Self::UntilLastClientDisconnects)
    }

    /// Affirmative: only a launcher-created child is bound to a parent
    /// control connection.
    pub fn is_launch_bound(self) -> bool {
        matches!(self, Self::LaunchBound)
    }
}

#[cfg(test)]
#[path = "harness_lifetime_tests.rs"]
mod tests;
