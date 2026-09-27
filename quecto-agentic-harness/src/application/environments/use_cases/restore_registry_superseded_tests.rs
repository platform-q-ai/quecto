//! #2190: a correction another quecto process overtook is reported as the
//! race it was — the status this restore loaded, then the status on file —
//! only when the other process's verdict differs from this restore's, and
//! to the audience the restore speaks for.
use std::sync::Arc;

use super::super::dto::{CorrectionOutcome, EnvironmentLiveness, OvertakenAudience, RestoreMode};
use super::super::ports::EnvironmentRegistryStore;
use super::RestoreRegistry;
use super::restore_registry_tests::{FakeStore, no_hosted, process, record, store_with};
use crate::domain::environment_registry::{EnvironmentRecord, EnvironmentStatus};

/// The real store's race: `load` returns what is on file, then another
/// quecto process writes `written` before this restore corrects. The
/// conditional correction is the store's own (applied only while the
/// loaded status is still on file).
struct RacingStore {
    inner: Arc<FakeStore>,
    written: EnvironmentRecord,
}

impl EnvironmentRegistryStore for RacingStore {
    fn allocate_ref(&self, floor: u64) -> Result<u64, String> {
        self.inner.allocate_ref(floor)
    }
    fn release_ref(&self, number: u64) -> Result<(), String> {
        self.inner.release_ref(number)
    }
    fn load(&self) -> Result<Vec<EnvironmentRecord>, String> {
        let loaded = self.inner.load()?;
        self.inner.record(&self.written)?;
        Ok(loaded)
    }
    fn record(&self, record: &EnvironmentRecord) -> Result<(), String> {
        self.inner.record(record)
    }
    fn correct(
        &self,
        record: &EnvironmentRecord,
        expected: &EnvironmentStatus,
    ) -> Result<CorrectionOutcome, String> {
        self.inner.correct(record, expected)
    }
    fn forget(&self, record: &EnvironmentRecord) -> Result<(), String> {
        self.inner.forget(record)
    }
}

/// C1, created by `cli:one` and loaded `running`, is judged gone (so
/// `stopped`) here while another process writes `written` over it; the
/// restore runs as `session` for `audience`.
fn restore_racing(
    written: EnvironmentRecord,
    session: &str,
    audience: OvertakenAudience,
) -> Vec<String> {
    let inner = store_with(vec![record("C1", EnvironmentStatus::Running)]);
    let store = RacingStore {
        inner: inner.clone(),
        written: written.clone(),
    };
    let (registry, report) = RestoreRegistry::new(
        Arc::new(store),
        process(|_| EnvironmentLiveness::Gone),
        no_hosted(),
    )
    .restore(session, RestoreMode::Correct, audience);
    assert!(
        inner.corrections.lock().unwrap().is_empty(),
        "the overtaken correction is not written"
    );
    assert_eq!(
        registry.get("C1").map(|seeded| seeded.status),
        Some(written.status),
        "what the other process wrote is seeded"
    );
    report.diagnostics
}

fn retained_elsewhere() -> EnvironmentRecord {
    let mut written = record("C1", EnvironmentStatus::Running);
    written.retain_with("kept by its owner");
    written
}

const OVERTAKEN: &str = "C1 changed while it was being checked (running → retained): \
    another quecto process changed it; its state stands";

#[test]
fn an_overtaken_correction_of_an_own_environment_shows_loaded_then_current() {
    let diagnostics = restore_racing(
        retained_elsewhere(),
        "cli:one",
        OvertakenAudience::OwnEnvironments,
    );
    assert_eq!(diagnostics, [OVERTAKEN]);
}

#[test]
fn an_overtaken_correction_that_agrees_with_this_verdict_is_not_reported() {
    // The other process stopped C1 too: both verdicts agree.
    for audience in [OvertakenAudience::OwnEnvironments, OvertakenAudience::Fleet] {
        let diagnostics = restore_racing(
            record("C1", EnvironmentStatus::Stopped),
            "cli:one",
            audience,
        );
        assert!(diagnostics.is_empty(), "{audience:?}: {diagnostics:?}");
    }
}

#[test]
fn a_session_leaves_another_sessions_overtaken_correction_off_stderr() {
    for session in ["cli:other", ""] {
        let diagnostics = restore_racing(
            retained_elsewhere(),
            session,
            OvertakenAudience::OwnEnvironments,
        );
        assert!(diagnostics.is_empty(), "{session}: {diagnostics:?}");
    }
}

/// L4: `quecto container ls|kill|gc` restores as `cli`, which created none
/// of the environments it lists; it speaks for the fleet and reports all.
#[test]
fn a_fleet_wide_command_reports_every_overtaken_correction() {
    for session in ["cli", "cli:other", ""] {
        let diagnostics = restore_racing(retained_elsewhere(), session, OvertakenAudience::Fleet);
        assert_eq!(diagnostics, [OVERTAKEN], "{session}");
    }
}
