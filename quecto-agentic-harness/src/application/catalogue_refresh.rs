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
    CatalogueSnapshotStore, CatalogueSource, CredentialStatusPort, ResolveCatalogueUseCase,
    ResolvedCatalogue,
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
        ports: &RefreshPorts<'_>,
        selection: &RefreshSelection,
        ctx: &RefreshContext,
    ) -> CatalogueRefreshReport {
        let mut outcomes = Vec::new();
        for target in selected_targets(ports.refreshables, selection, &mut outcomes) {
            outcomes.push(SourceRefreshOutcome {
                source: target.id().to_string(),
                status: refresh_one(target, ctx, ports.redaction),
            });
        }
        // Republish only when a source actually changed its cached data: an
        // all-unchanged or all-failed run keeps the previous valid generation
        // published rather than minting an identical (or degraded) successor.
        let any_updated = outcomes
            .iter()
            .any(|o| matches!(o.status, SourceRefreshStatus::Updated { .. }));
        let resolved = any_updated.then(|| {
            ResolveCatalogueUseCase.resolve_and_publish(
                ports.sources,
                ports.credentials,
                ports.store,
            )
        });
        CatalogueRefreshReport { outcomes, resolved }
    }
}

/// The refreshables this run targets, in configured order. A selected name
/// with no matching refreshable source is reported as a failed outcome (never
/// silently dropped), so a typo in a subset refresh is visible.
fn selected_targets<'a>(
    refreshables: &'a [&'a dyn RefreshableCatalogueSource],
    selection: &RefreshSelection,
    outcomes: &mut Vec<SourceRefreshOutcome>,
) -> Vec<&'a dyn RefreshableCatalogueSource> {
    match selection {
        RefreshSelection::All => refreshables.to_vec(),
        RefreshSelection::Only(names) => {
            let mut targets = Vec::new();
            for name in names {
                match refreshables.iter().find(|s| s.id() == name) {
                    Some(source) => targets.push(*source),
                    None => outcomes.push(SourceRefreshOutcome {
                        source: name.clone(),
                        status: SourceRefreshStatus::Failed {
                            reason: format!("no refreshable catalogue source named '{name}'"),
                        },
                    }),
                }
            }
            targets
        }
    }
}

/// Refresh one source under the run's bounds. A cancellation observed before
/// the source starts skips it as `Cancelled`; a refresh that outlives the
/// timeout is reported failed even if it eventually returned, because its
/// result arrived outside the budget an unattended caller allowed for.
fn refresh_one(
    source: &dyn RefreshableCatalogueSource,
    ctx: &RefreshContext,
    redaction: &dyn RefreshRedactionPort,
) -> SourceRefreshStatus {
    if ctx.is_cancelled() {
        return SourceRefreshStatus::Cancelled;
    }
    let started = std::time::Instant::now();
    let result = source.refresh(ctx);
    // A cancellation the source observed mid-refresh is a cancellation, not a
    // timeout failure, even when the aborted attempt outlived the budget.
    if matches!(result, Err(RefreshError::Cancelled)) {
        return SourceRefreshStatus::Cancelled;
    }
    if started.elapsed() > ctx.bounds.timeout {
        return SourceRefreshStatus::Failed {
            reason: format!(
                "refresh exceeded the {}ms timeout",
                ctx.bounds.timeout.as_millis()
            ),
        };
    }
    match result {
        Ok(RefreshChange::Updated { models }) => SourceRefreshStatus::Updated { models },
        Ok(RefreshChange::Unchanged) => SourceRefreshStatus::Unchanged,
        Err(RefreshError::Unsupported { reason }) => SourceRefreshStatus::Unsupported {
            reason: redaction.redact(&reason),
        },
        Err(RefreshError::Failed { reason }) => SourceRefreshStatus::Failed {
            reason: redaction.redact(&reason),
        },
        Err(RefreshError::Cancelled) => SourceRefreshStatus::Cancelled,
    }
}

#[cfg(test)]
#[path = "catalogue_refresh_tests.rs"]
mod tests;
