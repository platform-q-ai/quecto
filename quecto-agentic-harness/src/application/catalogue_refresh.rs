//! Application catalogue refresh use case and ports (epic #1193, slice 4).
//!
//! Discovery is a *refreshable catalogue source*: infrastructure implements
//! [`RefreshableCatalogueSource`] (provider-specific HTTP plus a persisted
//! source cache); the application owns the one refresh use case that asks
//! sources to refresh, collects per-source outcomes, and republishes the
//! effective catalogue through the slice-2 resolve path. Interfaces (CLI,
//! UDS, TUI) are adapters over this use case and own no discovery semantics.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::application::catalogue::{
    CatalogueSnapshotStore, CatalogueSource, CredentialStatusPort, ResolvedCatalogue,
};

/// Hard bounds every remote refresh runs under, so unattended refreshes can
/// never hang forever or buffer an unbounded response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefreshBounds {
    pub timeout: Duration,
    pub max_response_bytes: u64,
}

impl Default for RefreshBounds {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            max_response_bytes: 4 * 1024 * 1024,
        }
    }
}

/// Per-run refresh context: bounds plus a cooperative cancellation flag.
#[derive(Debug, Clone, Default)]
pub struct RefreshContext {
    pub bounds: RefreshBounds,
    cancel: Arc<AtomicBool>,
}

impl RefreshContext {
    pub fn new(bounds: RefreshBounds) -> Self {
        Self {
            bounds,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
}

/// What a successful source refresh changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshChange {
    Updated { models: usize },
    Unchanged,
}

/// Why a source refresh did not succeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshError {
    /// The provider has no remote listing to refresh from; the reason must be
    /// actionable (what is unsupported and what the user can do instead).
    Unsupported {
        reason: String,
    },
    Failed {
        reason: String,
    },
    Cancelled,
}

/// Application port: a catalogue source that can be asked to refresh itself
/// from its remote origin. Ordinary loads stay network-free; network happens
/// only inside [`RefreshableCatalogueSource::refresh`].
pub trait RefreshableCatalogueSource: CatalogueSource {
    fn refresh(&self, ctx: &RefreshContext) -> Result<RefreshChange, RefreshError>;
}

/// Terminal status of one source in a refresh run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceRefreshStatus {
    Updated { models: usize },
    Unchanged,
    Unsupported { reason: String },
    Failed { reason: String },
    Cancelled,
}

/// One source's outcome, reported per source so a mixed run stays legible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRefreshOutcome {
    pub source: String,
    pub status: SourceRefreshStatus,
}

/// Which sources a refresh run targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshSelection {
    All,
    Only(Vec<String>),
}

/// Redaction port: strips credential material from human-facing refresh text.
/// Implemented by infrastructure (which knows the secret values); the
/// application never sees the secrets themselves.
pub trait RefreshRedactionPort: Send + Sync {
    fn redact(&self, text: &str) -> String;
}

/// A redactor for contexts with no credential material in scope.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopRedaction;

impl RefreshRedactionPort for NoopRedaction {
    fn redact(&self, text: &str) -> String {
        text.to_string()
    }
}

/// The full result of one refresh run: per-source outcomes plus the resolve
/// result when a republish happened (`None` when nothing was republished).
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogueRefreshReport {
    pub outcomes: Vec<SourceRefreshOutcome>,
    pub resolved: Option<ResolvedCatalogue>,
}

/// Ports the refresh use case drives.
pub struct RefreshPorts<'a> {
    /// Sources that can be refreshed remotely.
    pub refreshables: &'a [&'a dyn RefreshableCatalogueSource],
    /// The full precedence-ordered source set to resolve after refreshing
    /// (includes the refreshables' cached data plus every other layer).
    pub sources: &'a [&'a dyn CatalogueSource],
    pub credentials: &'a dyn CredentialStatusPort,
    pub store: &'a CatalogueSnapshotStore,
    pub redaction: &'a dyn RefreshRedactionPort,
}

/// Refresh all (or a selected subset of) refreshable catalogue sources in one
/// operation, collect per-source outcomes, then resolve and atomically
/// publish a new generation via the slice-2 path. One source's failure or
/// cancellation never discards other sources' successes or the previous
/// valid state.
#[derive(Debug, Default, Clone, Copy)]
pub struct RefreshCatalogueSourcesUseCase;

impl RefreshCatalogueSourcesUseCase {
    pub fn refresh(
        &self,
        _ports: &RefreshPorts<'_>,
        _selection: &RefreshSelection,
        _ctx: &RefreshContext,
    ) -> CatalogueRefreshReport {
        // RED skeleton (#1574): outcomes and republish are not implemented yet.
        CatalogueRefreshReport {
            outcomes: Vec::new(),
            resolved: None,
        }
    }
}

#[cfg(test)]
#[path = "catalogue_refresh_tests.rs"]
mod tests;
