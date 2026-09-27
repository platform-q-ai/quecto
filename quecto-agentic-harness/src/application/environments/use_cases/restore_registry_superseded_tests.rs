//! #2190: a correction another session overtook is reported as the race
//! it was — the status this session loaded, then the status on file —
//! only when those differ, and only for this session's own environments.
use std::sync::Arc;

use super::super::dto::{CorrectionOutcome, EnvironmentLiveness};
use super::super::ports::EnvironmentRegistryStore;
use super::RestoreRegistry;
use super::restore_registry_tests::{FakeStore, no_hosted, process, record, store_with};
use crate::domain::environment_registry::{EnvironmentRecord, EnvironmentStatus};

/// Loads what `inner` holds, then answers every correction with `current`:
/// what another session wrote between this session's load and its write.
struct OvertakenStore {
    inner: Arc<FakeStore>,
    current: EnvironmentRecord,
}

impl EnvironmentRegistryStore for OvertakenStore {
    fn allocate_ref(&self, floor: u64) -> Result<u64, String> {
        self.inner.allocate_ref(floor)
    }
    fn release_ref(&self, number: u64) -> Result<(), String> {
        self.inner.release_ref(number)
    }
    fn load(&self) -> Result<Vec<EnvironmentRecord>, String> {
        self.inner.load()
    }
    fn record(&self, record: &EnvironmentRecord) -> Result<(), String> {
        self.inner.record(record)
    }
    fn correct(
        &self,
        _record: &EnvironmentRecord,
        _expected: &EnvironmentStatus,
    ) -> Result<CorrectionOutcome, String> {
        Ok(CorrectionOutcome::Superseded(Box::new(
            self.current.clone(),
        )))
    }
    fn forget(&self, record: &EnvironmentRecord) -> Result<(), String> {
        self.inner.forget(record)
    }
}

/// C1, created by `cli:one` and loaded `running`, judged gone here, while
/// another session wrote `current` over it; restored as `session`.
fn restore_overtaken(current: EnvironmentRecord, session: &str) -> Vec<String> {
    let store = OvertakenStore {
        inner: store_with(vec![record("C1", EnvironmentStatus::Running)]),
        current,
    };
    let (registry, report) = RestoreRegistry::new(
        Arc::new(store),
        process(|_| EnvironmentLiveness::Gone),
        no_hosted(),
    )
    .execute(session);
    assert!(
        registry.get("C1").is_some(),
        "what the other session wrote is seeded"
    );
    report.diagnostics
}

#[test]
fn an_overtaken_correction_of_an_own_environment_shows_loaded_then_current() {
    let diagnostics = restore_overtaken(record("C1", EnvironmentStatus::Stopped), "cli:one");
    assert_eq!(
        diagnostics,
        [
            "C1 changed while it was being checked (running → stopped); \
          the other session's state stands"
        ]
    );
}

#[test]
fn an_overtaken_correction_that_changed_no_status_is_not_reported() {
    // The other session wrote the status this session loaded: nothing
    // changed that a reader could see.
    let diagnostics = restore_overtaken(record("C1", EnvironmentStatus::Running), "cli:one");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn an_overtaken_correction_of_another_sessions_environment_stays_off_stderr() {
    for session in ["cli:other", ""] {
        let diagnostics = restore_overtaken(record("C1", EnvironmentStatus::Stopped), session);
        assert!(diagnostics.is_empty(), "{session}: {diagnostics:?}");
    }
}
