//! Outbound boundary for resolving a launch directory into session-home scope facts.

use std::path::Path;

use crate::domain::session_scope::SessionHomeScope;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeDiscoveryOutcome {
    Discovered(SessionHomeScope),
    Unavailable { reason: String },
    Ambiguous { reason: String },
}

/// Application-facing workspace discovery. Implementations may inspect the
/// filesystem and Git; callers only receive affirmative, typed outcomes.
pub trait SessionScopeDiscovery: Send + Sync {
    fn discover(&self, directory: &Path) -> ScopeDiscoveryOutcome;
}
