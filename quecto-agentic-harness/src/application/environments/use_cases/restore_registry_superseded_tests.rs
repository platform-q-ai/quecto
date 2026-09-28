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

/// Everything `run` logs at debug and above, as text.
fn captured_debug_log(run: impl FnOnce()) -> String {
    #[derive(Clone, Default)]
    struct Captured(Arc<std::sync::Mutex<String>>);
    impl std::io::Write for Captured {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap()
                .push_str(&String::from_utf8_lossy(buf));
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
        type Writer = Captured;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }
    let captured = Captured::default();
    let sink = captured.0.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured)
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .finish();
    tracing::subscriber::with_default(subscriber, run);
    sink.lock().unwrap().clone()
}

/// #2247 review: another process stopped C1 as this restore would, but for
/// its own reason. The verdicts agree, so stderr stays silent (#2190); the
/// differing account is kept in the debug log, naming what differs.
#[test]
fn an_agreeing_correction_with_a_different_account_is_logged_at_debug() {
    let mut written = record("C1", EnvironmentStatus::Stopped);
    written.last_error = Some("killed by its owner".into());
    let mut diagnostics = Vec::new();
    let log = captured_debug_log(|| {
        diagnostics = restore_racing(written, "cli:one", OvertakenAudience::OwnEnvironments);
    });
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let line = log
        .lines()
        .find(|line| line.contains("C1") && line.contains("agrees"))
        .unwrap_or_else(|| panic!("no debug line: {log}"));
    assert!(line.contains("DEBUG"), "{line}");
    assert!(line.contains("killed by its owner"), "{line}");
}

/// An agreeing correction whose account is this restore's own says nothing.
#[test]
fn an_agreeing_correction_with_the_same_account_is_not_logged() {
    let mut written = record("C1", EnvironmentStatus::Stopped);
    written.last_error = Some(crate::domain::environment_registry::GONE_AT_RESTORE.into());
    let log = captured_debug_log(|| {
        restore_racing(written, "cli:one", OvertakenAudience::OwnEnvironments);
    });
    assert!(!log.contains("agrees"), "{log}");
}
