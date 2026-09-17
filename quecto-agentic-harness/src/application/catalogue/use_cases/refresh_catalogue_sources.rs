//! Refresh model catalogue sources (#1846) and discover a provider's models
//! (#1844): refresh all (or a selected subset of) refreshable sources in one
//! operation, collect per-source outcomes, then resolve and atomically
//! publish a new generation. One source's failure or cancellation never
//! discards other sources' successes or the previous valid state; a run
//! that changed nothing republishes nothing. Network happens only here —
//! ordinary catalogue reads stay network-free.

use std::sync::Arc;

use crate::application::catalogue::dto::{
    CatalogueRefreshReport, REGISTRY_FILE_SOURCE, RefreshBounds, RefreshSelection,
    SourceRefreshOutcome, SourceRefreshStatus,
};
use crate::application::catalogue::ports::{
    LoadedRefreshInputs, RefreshChange, RefreshContext, RefreshError, RefreshInputsLoader,
    RefreshRedactionPort, RefreshableCatalogueSource,
};
use crate::application::catalogue::{CatalogueSnapshotStore, ResolveCatalogueUseCase};

pub struct RefreshCatalogueSources {
    inputs: Arc<dyn RefreshInputsLoader>,
    store: CatalogueSnapshotStore,
}

impl RefreshCatalogueSources {
    pub fn new(inputs: Arc<dyn RefreshInputsLoader>, store: CatalogueSnapshotStore) -> Self {
        Self { inputs, store }
    }

    /// Refresh `selection` under `bounds` and report per source. No entry
    /// point cancels a run today; cooperative cancellation is exercised at
    /// the [`refresh_loaded`](Self::refresh_loaded) seam.
    pub fn execute(
        &self,
        selection: &RefreshSelection,
        bounds: RefreshBounds,
    ) -> CatalogueRefreshReport {
        let ctx = RefreshContext::new(bounds);
        let loaded = match self.inputs.load() {
            Ok(loaded) => loaded,
            // A catalogue file that cannot be enumerated is one failed
            // outcome, not a crash: the previous valid snapshot stays
            // published.
            Err(reason) => {
                return CatalogueRefreshReport {
                    outcomes: vec![SourceRefreshOutcome {
                        source: REGISTRY_FILE_SOURCE.to_string(),
                        status: SourceRefreshStatus::Failed { reason },
                    }],
                    resolved: None,
                };
            }
        };
        self.refresh_loaded(loaded.as_ref(), selection, &ctx)
    }

    /// Refresh over inputs already loaded (the loader's one read, or a
    /// rig's fakes): the whole policy of a run, independent of loading.
    /// Public so the use-case rigs (unit and contract tests) can drive the
    /// policy over fakes without a loader.
    pub fn refresh_loaded(
        &self,
        inputs: &dyn LoadedRefreshInputs,
        selection: &RefreshSelection,
        ctx: &RefreshContext,
    ) -> CatalogueRefreshReport {
        let refreshables = inputs.refreshables();
        let mut outcomes = Vec::new();
        for target in selected_targets(&refreshables, selection, &mut outcomes) {
            outcomes.push(SourceRefreshOutcome {
                source: target.id().to_string(),
                status: refresh_one(target, ctx, inputs.redaction()),
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
                &inputs.sources(),
                inputs.credentials(),
                &self.store,
            )
        });
        CatalogueRefreshReport { outcomes, resolved }
    }
}

impl std::fmt::Debug for RefreshCatalogueSources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RefreshCatalogueSources")
            .finish_non_exhaustive()
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
/// the source starts skips it as `Cancelled`; a side-effect-free refresh that
/// outlives the timeout is reported failed even if it eventually returned,
/// because its result arrived outside the budget an unattended caller allowed
/// for (an over-budget *update* keeps its Updated outcome — see below).
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
    // An over-budget refresh that persisted an update is still reported
    // Updated: the new cache is on disk and will be resolved either way, so
    // reclassifying it as failed would suppress the republish and report a
    // state the next resolve contradicts (slice-4 review). Only outcomes with
    // no new side effects are reclassified as timeout failures.
    if !matches!(result, Ok(RefreshChange::Updated { .. }))
        && started.elapsed() > ctx.bounds.timeout
    {
        return SourceRefreshStatus::Failed {
            reason: format!(
                "refresh exceeded the {}ms timeout",
                ctx.bounds.timeout.as_millis()
            ),
        };
    }
    match result {
        Ok(RefreshChange::Updated { models }) => SourceRefreshStatus::Updated { models },
        Ok(RefreshChange::Unchanged { models }) => SourceRefreshStatus::Unchanged { models },
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
#[path = "refresh_catalogue_sources_bounds_tests.rs"]
mod bounds_tests;
#[cfg(test)]
#[path = "refresh_catalogue_sources_tests.rs"]
mod tests;
