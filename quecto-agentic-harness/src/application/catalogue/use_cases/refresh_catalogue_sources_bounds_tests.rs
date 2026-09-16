//! `RefreshCatalogueSources` bounds, cancellation, republish policy and the
//! loader seam (#1846); the run policy over fake ports lives in
//! `refresh_catalogue_sources_tests.rs`, whose fakes this file shares.

use std::sync::Arc;
use std::time::Duration;

use super::tests::{
    AllowAllCredentials, Behaviour, BorrowedInputs, FailingLoader, FakeRefreshable, StaticSource,
    entry, outcome, prepublish, run, use_case,
};
use super::*;
use crate::application::catalogue::ports::NoopRedaction;
use crate::application::catalogue::{CatalogueSource, CredentialStatusPort, SourceEntries};
use crate::domain::catalogue::SourceLayer;

/// A fake whose refresh sleeps for a fixed time before reporting an update,
/// for exercising both sides of the timeout bound.
struct SleepyRefreshable {
    id: String,
    sleep: Duration,
}

impl CatalogueSource for SleepyRefreshable {
    fn id(&self) -> &str {
        &self.id
    }
    fn layer(&self) -> SourceLayer {
        SourceLayer::Discovered
    }
    fn load(&self) -> Result<SourceEntries, String> {
        Ok(SourceEntries::default())
    }
}

impl RefreshableCatalogueSource for SleepyRefreshable {
    fn refresh(&self, _ctx: &RefreshContext) -> Result<RefreshChange, RefreshError> {
        std::thread::sleep(self.sleep);
        Ok(RefreshChange::Unchanged { models: 0 })
    }
}

fn sleepy_report(sleep: Duration, timeout: Duration) -> CatalogueRefreshReport {
    let slow = SleepyRefreshable {
        id: "slow".to_string(),
        sleep,
    };
    let store = CatalogueSnapshotStore::empty();
    let refreshables: Vec<&dyn RefreshableCatalogueSource> = vec![&slow];
    let sources: Vec<&dyn CatalogueSource> = vec![&slow];
    let inputs = BorrowedInputs {
        refreshables: &refreshables,
        sources: &sources,
        redaction: &NoopRedaction,
    };
    use_case(&store).refresh_loaded(
        &inputs,
        &RefreshSelection::All,
        &RefreshContext::new(RefreshBounds {
            timeout,
            ..RefreshBounds::default()
        }),
    )
}

#[test]
fn refresh_outliving_the_timeout_is_reported_failed() {
    let report = sleepy_report(Duration::from_millis(80), Duration::from_millis(20));
    match &outcome(&report, "slow").status {
        SourceRefreshStatus::Failed { reason } => {
            assert!(reason.contains("timeout"), "got: {reason}");
        }
        other => panic!("an over-timeout refresh must be failed, got {other:?}"),
    }
}

#[test]
fn cancellation_observed_mid_refresh_is_not_reclassified_as_timeout() {
    struct SlowCancelled {
        sleep: Duration,
    }
    impl CatalogueSource for SlowCancelled {
        fn id(&self) -> &str {
            "slow-cancelled"
        }
        fn layer(&self) -> SourceLayer {
            SourceLayer::Discovered
        }
        fn load(&self) -> Result<SourceEntries, String> {
            Ok(SourceEntries::default())
        }
    }
    impl RefreshableCatalogueSource for SlowCancelled {
        fn refresh(&self, _ctx: &RefreshContext) -> Result<RefreshChange, RefreshError> {
            std::thread::sleep(self.sleep);
            Err(RefreshError::Cancelled)
        }
    }
    let slow = SlowCancelled {
        sleep: Duration::from_millis(50),
    };
    let store = CatalogueSnapshotStore::empty();
    let refreshables: Vec<&dyn RefreshableCatalogueSource> = vec![&slow];
    let sources: Vec<&dyn CatalogueSource> = vec![&slow];
    let inputs = BorrowedInputs {
        refreshables: &refreshables,
        sources: &sources,
        redaction: &NoopRedaction,
    };
    let report = use_case(&store).refresh_loaded(
        &inputs,
        &RefreshSelection::All,
        &RefreshContext::new(RefreshBounds {
            timeout: Duration::from_millis(10),
            ..RefreshBounds::default()
        }),
    );
    assert_eq!(
        outcome(&report, "slow-cancelled").status,
        SourceRefreshStatus::Cancelled,
        "a cancellation the source observed must never surface as a timeout failure"
    );
}

#[test]
fn refresh_within_the_timeout_keeps_its_own_outcome() {
    let report = sleepy_report(Duration::from_millis(1), Duration::from_secs(5));
    assert_eq!(
        outcome(&report, "slow").status,
        SourceRefreshStatus::Unchanged { models: 0 },
        "a refresh inside the timeout must not be reclassified"
    );
}

#[test]
fn all_unchanged_run_republishes_nothing() {
    let seed = StaticSource {
        id: "seed".to_string(),
        layer: SourceLayer::BuiltIn,
        entries: vec![entry("openai-api/gpt-5", "GPT 5")],
    };
    let local = FakeRefreshable::new("local", Behaviour::Unchanged);
    let store = CatalogueSnapshotStore::empty();
    prepublish(&store, &[&seed as &dyn CatalogueSource]);
    assert_eq!(store.current().generation(), 1);

    let report = run(
        &[&local],
        &[&seed],
        &store,
        &RefreshSelection::All,
        &RefreshContext::default(),
        &NoopRedaction,
    );

    assert_eq!(
        outcome(&report, "local").status,
        SourceRefreshStatus::Unchanged { models: 0 }
    );
    assert!(
        report.resolved.is_none(),
        "an all-unchanged run must not republish"
    );
    assert_eq!(
        store.current().generation(),
        1,
        "an all-unchanged run keeps the previous generation published"
    );
}

#[test]
fn selecting_an_unknown_source_reports_a_failed_outcome() {
    let local = FakeRefreshable::new("local", Behaviour::Unchanged);
    let store = CatalogueSnapshotStore::empty();

    let report = run(
        &[&local],
        &[],
        &store,
        &RefreshSelection::Only(vec!["nonexistent".to_string()]),
        &RefreshContext::default(),
        &NoopRedaction,
    );

    match &outcome(&report, "nonexistent").status {
        SourceRefreshStatus::Failed { reason } => {
            assert!(reason.contains("nonexistent"), "got: {reason}");
        }
        other => panic!("an unknown selected source must be failed, got {other:?}"),
    }
    assert_eq!(local.calls(), 0, "no configured source may be refreshed");
    assert!(report.resolved.is_none());
}

/// An owned loader over fakes, for the `execute` seam.
struct StaticLoader {
    refreshables: Vec<Arc<FakeRefreshable>>,
}

struct StaticLoaded(Vec<Arc<FakeRefreshable>>);

impl LoadedRefreshInputs for StaticLoaded {
    fn refreshables(&self) -> Vec<&dyn RefreshableCatalogueSource> {
        self.0
            .iter()
            .map(|s| s.as_ref() as &dyn RefreshableCatalogueSource)
            .collect()
    }
    fn sources(&self) -> Vec<&dyn CatalogueSource> {
        self.0
            .iter()
            .map(|s| s.as_ref() as &dyn CatalogueSource)
            .collect()
    }
    fn credentials(&self) -> &dyn CredentialStatusPort {
        &AllowAllCredentials
    }
    fn redaction(&self) -> &dyn RefreshRedactionPort {
        &NoopRedaction
    }
}

impl RefreshInputsLoader for StaticLoader {
    fn load(&self) -> Result<Box<dyn LoadedRefreshInputs>, String> {
        Ok(Box::new(StaticLoaded(self.refreshables.clone())))
    }
}

#[test]
fn execute_loads_the_inputs_and_runs_the_refresh() {
    let openrouter = Arc::new(FakeRefreshable::new(
        "openrouter",
        Behaviour::Update(vec![
            entry("openrouter/alpha", "Alpha"),
            entry("openrouter/beta", "Beta"),
        ]),
    ));
    let store = CatalogueSnapshotStore::empty();
    let report = RefreshCatalogueSources::new(
        Arc::new(StaticLoader {
            refreshables: vec![openrouter.clone()],
        }),
        store.clone(),
    )
    .execute(&RefreshSelection::All, RefreshBounds::default());
    assert_eq!(
        outcome(&report, "openrouter").status,
        SourceRefreshStatus::Updated { models: 2 }
    );
    assert_eq!(openrouter.calls(), 1);
    assert_eq!(
        store.current().generation(),
        1,
        "the update was republished"
    );
}

#[test]
fn a_catalogue_file_that_cannot_be_enumerated_is_one_failed_outcome() {
    let store = CatalogueSnapshotStore::empty();
    let report = RefreshCatalogueSources::new(
        Arc::new(FailingLoader(
            "models.json: expected value at line 1".into(),
        )),
        store.clone(),
    )
    .execute(&RefreshSelection::All, RefreshBounds::default());
    assert_eq!(
        report.outcomes,
        vec![SourceRefreshOutcome {
            source: REGISTRY_FILE_SOURCE.to_string(),
            status: SourceRefreshStatus::Failed {
                reason: "models.json: expected value at line 1".into()
            },
        }]
    );
    assert!(report.resolved.is_none());
    assert_eq!(store.current().generation(), 0, "the previous state stays");
}
