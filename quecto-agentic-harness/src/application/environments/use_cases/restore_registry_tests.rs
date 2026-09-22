use std::sync::{Arc, Mutex};

use super::super::dto::{CorrectionOutcome, EnvironmentLiveness};
use super::super::ports::{EnvironmentProcess, EnvironmentRegistryStore, HostedSwarmRunInspection};
use super::{GONE_AT_RESTORE, KILL_IN_FLIGHT, RETAINED_EXITED, RestoreRegistry};
use crate::domain::environment_registry::{
    EnvironmentOrigin, EnvironmentRecord, EnvironmentStatus,
};
use crate::domain::environment_retention::SwarmRunObservation;

/// The hosted store as the restore reads it: one observation per ref
/// (`NoStore` for any other), and which refs were asked.
pub(super) struct FakeHosted {
    pub(super) by_ref: Mutex<Vec<(String, SwarmRunObservation)>>,
    pub(super) asked: Mutex<Vec<String>>,
}

impl HostedSwarmRunInspection for FakeHosted {
    fn inspect_hosted_run(&self, record: &EnvironmentRecord) -> SwarmRunObservation {
        self.asked
            .lock()
            .unwrap()
            .push(record.environment_ref.clone());
        self.by_ref
            .lock()
            .unwrap()
            .iter()
            .find(|(r, _)| r == &record.environment_ref)
            .map(|(_, o)| o.clone())
            .unwrap_or(SwarmRunObservation::NoStore)
    }
    fn inspect_hosted_run_at(&self, _state_dir: &std::path::Path) -> SwarmRunObservation {
        panic!("the restore never judges a bare state dir")
    }
}

/// No environment hosts a store.
pub(super) fn no_hosted() -> Arc<FakeHosted> {
    Arc::new(FakeHosted {
        by_ref: Mutex::new(vec![]),
        asked: Mutex::new(vec![]),
    })
}

#[derive(Default)]
pub(super) struct FakeStore {
    pub(super) records: Mutex<Vec<EnvironmentRecord>>,
    next: Mutex<u64>,
    fail_load: std::sync::atomic::AtomicBool,
    fail_writes: bool,
    forgotten: Mutex<Vec<String>>,
    pub(super) corrections: Mutex<Vec<String>>,
}

impl EnvironmentRegistryStore for FakeStore {
    fn release_ref(&self, _: u64) -> Result<(), String> {
        Ok(())
    }

    fn allocate_ref(&self) -> Result<u64, String> {
        if self.fail_writes {
            return Err("disk full".into());
        }
        let highest = self
            .records
            .lock()
            .unwrap()
            .iter()
            .filter_map(|r| crate::domain::environment_registry::ref_number(&r.environment_ref))
            .max()
            .unwrap_or(0);
        let mut next = self.next.lock().unwrap();
        *next = (*next).max(highest) + 1;
        Ok(*next)
    }
    fn load(&self) -> Result<Vec<EnvironmentRecord>, String> {
        if self.fail_load.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("corrupt".into());
        }
        Ok(self.records.lock().unwrap().clone())
    }
    fn record(&self, record: &EnvironmentRecord) -> Result<(), String> {
        if self.fail_writes {
            return Err("disk full".into());
        }
        let mut records = self.records.lock().unwrap();
        records.retain(|r| r.environment_ref != record.environment_ref);
        records.push(record.clone());
        Ok(())
    }
    fn correct(
        &self,
        record: &EnvironmentRecord,
        expected: &EnvironmentStatus,
    ) -> Result<CorrectionOutcome, String> {
        if self.fail_writes {
            return Err("disk full".into());
        }
        let mut records = self.records.lock().unwrap();
        let Some(current) = records
            .iter_mut()
            .find(|r| r.environment_ref == record.environment_ref)
        else {
            return Ok(CorrectionOutcome::Forgotten);
        };
        if current.status != *expected {
            return Ok(CorrectionOutcome::Superseded(Box::new(current.clone())));
        }
        *current = record.clone();
        self.corrections
            .lock()
            .unwrap()
            .push(record.environment_ref.clone());
        Ok(CorrectionOutcome::Applied)
    }

    fn forget(&self, environment_ref: &str) -> Result<(), String> {
        if self.fail_writes {
            return Err("disk full".into());
        }
        self.forgotten.lock().unwrap().push(environment_ref.into());
        self.records
            .lock()
            .unwrap()
            .retain(|r| r.environment_ref != environment_ref);
        Ok(())
    }
}

struct FakeProcess(Box<dyn Fn(&EnvironmentRecord) -> EnvironmentLiveness + Send + Sync>);

impl EnvironmentProcess for FakeProcess {
    fn observe(&self, record: &EnvironmentRecord) -> EnvironmentLiveness {
        (self.0)(record)
    }
    fn cleanup(&self, _record: &EnvironmentRecord) -> Result<(), String> {
        Ok(())
    }
}

pub(super) fn record(reference: &str, status: EnvironmentStatus) -> EnvironmentRecord {
    EnvironmentRecord {
        environment_ref: reference.into(),
        environment_id: format!("env-{reference}"),
        environment_uuid: format!("uuid-{reference}"),
        name: None,
        workspace_path: "/w".into(),
        repository: String::new(),
        script_name: "default".into(),
        retained_exec_argv: vec!["exec".into()],
        retained_kill_argv: vec!["kill".into()],
        retained_cleanup_argv: vec!["cleanup".into()],
        retained_inspect_argv: vec!["inspect".into()],
        members: vec!["old-member".into()],
        status,
        metadata: serde_json::json!({}),
        last_error: None,
        origin: EnvironmentOrigin::Created,
        created_by: "cli:one".into(),
        created_at: Some(1),
    }
}

pub(super) fn store_with(records: Vec<EnvironmentRecord>) -> Arc<FakeStore> {
    let store = FakeStore::default();
    *store.records.lock().unwrap() = records;
    Arc::new(store)
}

pub(super) fn process(
    f: impl Fn(&EnvironmentRecord) -> EnvironmentLiveness + Send + Sync + 'static,
) -> Arc<dyn EnvironmentProcess> {
    Arc::new(FakeProcess(Box::new(f)))
}

#[test]
fn live_records_are_restored_without_members_gone_ones_stopped_and_unknown_kept() {
    let store = store_with(vec![
        record("C1", EnvironmentStatus::Running),
        record("C2", EnvironmentStatus::Running),
        record("C3", EnvironmentStatus::CleanupFailed),
        record("C4", EnvironmentStatus::Stopped),
    ]);
    let process = process(|record| match record.environment_ref.as_str() {
        "C1" => EnvironmentLiveness::Running,
        "C2" => EnvironmentLiveness::Gone,
        "C3" => EnvironmentLiveness::Unknown("script missing".into()),
        other => panic!("stopped records are never inspected: {other}"),
    });
    let (registry, report) =
        RestoreRegistry::new(store.clone(), process, no_hosted()).execute("cli:two");
    assert_eq!(report.restored, ["C1"]);
    assert_eq!(report.stopped, ["C2"]);
    assert_eq!(
        report.unverified,
        [("C3".to_string(), "script missing".to_string())]
    );
    assert!(report.diagnostics.is_empty());
    assert_eq!(registry.session(), "cli:two");
    assert!(registry.is_durable());
    let c1 = registry.get("C1").unwrap();
    assert_eq!(c1.origin, EnvironmentOrigin::Restored);
    assert!(c1.members.is_empty());
    assert_eq!(c1.created_by, "cli:one");
    assert_eq!(c1.status_label(), "empty");
    let c2 = registry.get("C2").unwrap();
    assert_eq!(c2.status, EnvironmentStatus::Stopped);
    assert_eq!(c2.last_error.as_deref(), Some(GONE_AT_RESTORE));
    assert_eq!(
        registry.get("C3").unwrap().status,
        EnvironmentStatus::CleanupFailed
    );
    assert_eq!(
        registry.get("C4").unwrap().status,
        EnvironmentStatus::Stopped
    );
    // Only the corrected record was written, conditionally; nothing else
    // was written back.
    assert_eq!(store.corrections.lock().unwrap().as_slice(), ["C2"]);
    let on_file = store.load().unwrap();
    let c2 = on_file.iter().find(|r| r.environment_ref == "C2").unwrap();
    assert_eq!(c2.status, EnvironmentStatus::Stopped);
    assert_eq!(
        on_file
            .iter()
            .find(|r| r.environment_ref == "C1")
            .unwrap()
            .members,
        ["old-member"],
        "a seeded record is never written back"
    );
}

/// Review F6 (#2033): a `killing` record is reported, never rewritten —
/// its session may still be live and settling the kill; nothing here can
/// tell. An explicit kill from this session retries it.
#[test]
fn a_kill_in_flight_is_reported_not_relabelled_and_an_explicit_kill_retries_it() {
    let store = store_with(vec![record("C1", EnvironmentStatus::Killing)]);
    let process = process(|_| panic!("a killing record is not inspected"));
    let (registry, report) = RestoreRegistry::new(store.clone(), process, no_hosted()).execute("s");
    assert!(report.restored.is_empty(), "{report:?}");
    assert_eq!(report.unverified.len(), 1, "{report:?}");
    assert_eq!(report.unverified[0].0, "C1");
    assert_eq!(report.unverified[0].1, KILL_IN_FLIGHT);
    assert!(
        store.corrections.lock().unwrap().is_empty(),
        "nothing written"
    );
    let c1 = registry.get("C1").unwrap();
    assert_eq!(c1.status, EnvironmentStatus::Killing);
    assert_eq!(c1.last_error, None);
    assert_eq!(store.load().unwrap()[0].status, EnvironmentStatus::Killing);
    // The operator's explicit retry claims it (the other session's claim
    // is not visible here) and journals the outcome.
    let claim = registry
        .begin_kill("C1")
        .expect("a restored killing record is retryable");
    registry.complete_kill(claim);
    assert_eq!(store.load().unwrap()[0].status, EnvironmentStatus::Stopped);
}

#[test]
fn an_unreadable_store_yields_an_empty_registry_that_still_allocates_through_the_store() {
    let store = Arc::new(FakeStore {
        fail_load: true.into(),
        ..Default::default()
    });
    let process = process(|_| EnvironmentLiveness::Running);
    let (registry, report) = RestoreRegistry::new(store.clone(), process, no_hosted()).execute("s");
    assert!(registry.entries().is_empty());
    assert!(report.diagnostics.is_empty(), "{report:?}");
    let read_error = report.read_error.as_deref().expect("read error reported");
    assert!(read_error.contains("could not be read"), "{report:?}");
    assert!(read_error.contains("corrupt"), "{report:?}");
    assert_eq!(registry.mint_ref().unwrap(), "C1");
    assert_eq!(*store.next.lock().unwrap(), 1);
    // Round 2 F-B (#2033): the registry carries the error — a lookup of
    // a ref it could not have loaded answers with it, not `unknown`.
    assert_eq!(registry.read_error().as_deref(), Some(read_error));
    let lookup = registry
        .resolve(&crate::domain::environment_registry::EnvironmentTarget::Ref("C7".into()))
        .unwrap_err();
    assert_eq!(
        lookup.to_string(),
        format!("registry unreadable: {read_error}")
    );
}

#[test]
fn the_journal_writes_every_transition_and_forgets_a_rolled_back_create() {
    let store = store_with(vec![]);
    let process = process(|_| EnvironmentLiveness::Running);
    let (registry, _) = RestoreRegistry::new(store.clone(), process, no_hosted()).execute("s");
    let reference = registry.mint_ref().unwrap();
    assert_eq!(reference, "C1");
    let mut created = record(&reference, EnvironmentStatus::Running);
    created.members.clear();
    registry.commit(created);
    assert_eq!(store.load().unwrap().len(), 1);
    let claim = registry.begin_kill("C1").unwrap();
    assert_eq!(store.load().unwrap()[0].status, EnvironmentStatus::Killing);
    registry.fail_kill(claim, "boom");
    assert_eq!(
        store.load().unwrap()[0].status,
        EnvironmentStatus::CleanupFailed
    );
    let claim = registry.begin_kill("C1").unwrap();
    registry.complete_kill(claim);
    assert_eq!(store.load().unwrap()[0].status, EnvironmentStatus::Stopped);
    registry.remove("C1");
    assert_eq!(store.forgotten.lock().unwrap().as_slice(), ["C1"]);
    assert!(store.load().unwrap().is_empty());
}

/// Review F9 (#2033): a store that cannot allocate refuses the mint in
/// its own words; the rest of the journal fails soft (no panic).
#[test]
fn a_failing_store_never_panics_and_refuses_to_mint() {
    let store = Arc::new(FakeStore {
        fail_writes: true,
        ..Default::default()
    });
    let process = process(|_| EnvironmentLiveness::Running);
    let (registry, _) = RestoreRegistry::new(store, process, no_hosted()).execute("s");
    let refused = registry.mint_ref().unwrap_err();
    assert!(refused.to_string().contains("disk full"), "{refused}");
    assert!(registry.mint_ref().is_err());
    registry.commit(record("C2", EnvironmentStatus::Running));
    let claim = registry.begin_kill("C2").unwrap();
    registry.complete_kill(claim);
    assert!(registry.remove("C2").is_some());
}

#[test]
fn an_unseeded_registry_journals_but_inherits_nothing() {
    let store = store_with(vec![record("C5", EnvironmentStatus::Running)]);
    let process = process(|_| panic!("nothing is inspected"));
    let registry = RestoreRegistry::new(store.clone(), process, no_hosted()).unseeded("child");
    assert!(registry.entries().is_empty());
    assert_eq!(registry.session(), "child");
    // Refs still come from the shared store: no collision with C5.
    assert_eq!(registry.mint_ref().unwrap(), "C6");
    registry.commit(record("C6", EnvironmentStatus::Running));
    assert_eq!(store.load().unwrap().len(), 2);
}

#[test]
fn the_restore_debug_is_opaque_over_its_ports() {
    let store = store_with(vec![]);
    let process = process(|_| EnvironmentLiveness::Running);
    let shown = format!("{:?}", RestoreRegistry::new(store, process, no_hosted()));
    assert_eq!(shown, "RestoreRegistry { .. }");
}

#[test]
fn a_correction_another_session_overtook_is_not_written_and_their_state_is_seeded() {
    // The store's record moves to `stopped` (another session's kill)
    // between the load and the correction: the restore must not write its
    // own verdict over it, and seeds what is on file.
    struct MovingStore {
        inner: Arc<FakeStore>,
    }
    impl EnvironmentRegistryStore for MovingStore {
        fn allocate_ref(&self) -> Result<u64, String> {
            self.inner.allocate_ref()
        }
        fn release_ref(&self, number: u64) -> Result<(), String> {
            self.inner.release_ref(number)
        }
        fn load(&self) -> Result<Vec<EnvironmentRecord>, String> {
            let loaded = self.inner.load()?;
            // After the load, somebody else stops C1 and retains C2.
            let mut c1 = record("C1", EnvironmentStatus::Stopped);
            c1.last_error = Some("killed elsewhere".into());
            self.inner.record(&c1).unwrap();
            self.inner.forget("C2").unwrap();
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
        fn forget(&self, environment_ref: &str) -> Result<(), String> {
            self.inner.forget(environment_ref)
        }
    }
    let inner = store_with(vec![
        record("C1", EnvironmentStatus::Running),
        record("C2", EnvironmentStatus::Running),
    ]);
    let process = process(|_| EnvironmentLiveness::Gone);
    let (registry, report) = RestoreRegistry::new(
        Arc::new(MovingStore {
            inner: inner.clone(),
        }),
        process,
        no_hosted(),
    )
    .execute("s");
    assert!(inner.corrections.lock().unwrap().is_empty());
    let c1 = registry.get("C1").unwrap();
    assert_eq!(c1.last_error.as_deref(), Some("killed elsewhere"));
    assert!(
        registry.get("C2").is_none(),
        "a forgotten record is not resurrected"
    );
    assert_eq!(report.stopped, ["C1", "C2"]);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.contains("C1 changed while it was being checked")),
        "{report:?}"
    );
}

/// Round 3 H1 (#2033): a `retained` environment (#1924) is ended by an
/// explicit kill alone. Under the shipped adapter its container has
/// exited (the coordinator was PID 1), so "gone" is its normal state —
/// relabelling it `stopped` would hand its state dir (board, checkout,
/// unpushed branches) to the collector. It is reported, never rewritten.
#[test]
fn a_retained_record_whose_container_exited_stays_retained_and_is_reported() {
    let store = store_with(vec![record("C1", EnvironmentStatus::Retained)]);
    let process = process(|_| EnvironmentLiveness::Gone);
    let (registry, report) = RestoreRegistry::new(store.clone(), process, no_hosted()).execute("s");
    assert!(report.stopped.is_empty(), "{report:?}");
    assert!(report.restored.is_empty(), "{report:?}");
    assert_eq!(
        report.unverified,
        [("C1".to_string(), RETAINED_EXITED.to_string())]
    );
    assert!(
        store.corrections.lock().unwrap().is_empty(),
        "nothing written"
    );
    let c1 = registry.get("C1").unwrap();
    assert_eq!(c1.status, EnvironmentStatus::Retained);
    assert_eq!(c1.last_error, None);
    assert_eq!(store.load().unwrap()[0].status, EnvironmentStatus::Retained);
    // A retained record whose container still runs is restored as live.
    let store = store_with(vec![record("C2", EnvironmentStatus::Retained)]);
    let live = self::process(|_| EnvironmentLiveness::Running);
    let (_, report) = RestoreRegistry::new(store, live, no_hosted()).execute("s");
    assert_eq!(report.restored, ["C2"]);
}

/// Round 3 H1 (#2033): an observing restore (`gc --dry-run`) judges the
/// records the same way but writes nothing — the correction is seeded in
/// memory, so the dry run previews what a real run would do, and reported.
#[test]
fn an_observing_restore_seeds_its_corrections_in_memory_and_writes_none() {
    let store = store_with(vec![
        record("C1", EnvironmentStatus::Running),
        record("C2", EnvironmentStatus::Retained),
    ]);
    let process = process(|_| EnvironmentLiveness::Gone);
    let (registry, report) =
        RestoreRegistry::new(store.clone(), process, no_hosted()).observe("cli");
    assert_eq!(report.stopped, ["C1"]);
    assert!(
        report.diagnostics.iter().any(|d| d.starts_with("C1 ")
            && d.contains("would be recorded stopped")
            && d.contains("not written")),
        "{report:?}"
    );
    assert_eq!(
        registry.get("C1").unwrap().status,
        EnvironmentStatus::Stopped,
        "the preview judges as a real run would"
    );
    assert_eq!(
        registry.get("C2").unwrap().status,
        EnvironmentStatus::Retained
    );
    assert!(
        store.corrections.lock().unwrap().is_empty(),
        "nothing written"
    );
    let on_file = store.load().unwrap();
    assert_eq!(on_file[0].status, EnvironmentStatus::Running);
    assert_eq!(on_file[0].last_error, None);
    assert_eq!(on_file[1].status, EnvironmentStatus::Retained);
}

/// Round 3 L2 (#2033): the registry a session started with an unreadable
/// store retries the read on its next lookup. Once the document is
/// repaired in place, what it holds is seeded — judged and corrected as
/// at a restore — and `get_containers` stops reporting the stale error.
#[test]
fn a_store_repaired_in_place_is_seen_on_the_next_lookup_without_a_restart() {
    let store = store_with(vec![
        record("C1", EnvironmentStatus::Running),
        record("C2", EnvironmentStatus::Running),
    ]);
    store
        .fail_load
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let process = process(|record| match record.environment_ref.as_str() {
        "C1" => EnvironmentLiveness::Running,
        _ => EnvironmentLiveness::Gone,
    });
    let (registry, report) = RestoreRegistry::new(store.clone(), process, no_hosted()).execute("s");
    assert!(report.read_error.is_some());
    let query = super::ListEnvironmentsQuery::new(registry.clone());
    assert_eq!(query.diagnostics().len(), 1, "unreadable: reported");
    assert!(query.execute().is_empty());
    // Repaired.
    store
        .fail_load
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let listed: Vec<(String, EnvironmentStatus)> = query
        .execute()
        .into_iter()
        .map(|r| (r.environment_ref, r.status))
        .collect();
    assert_eq!(
        listed,
        [
            ("C1".to_string(), EnvironmentStatus::Running),
            ("C2".to_string(), EnvironmentStatus::Stopped)
        ],
        "seeded, judged against the runtime"
    );
    assert!(query.diagnostics().is_empty(), "the stale error is gone");
    assert_eq!(registry.read_error(), None);
    assert_eq!(
        store.corrections.lock().unwrap().as_slice(),
        ["C2"],
        "the late correction is written like a restore's"
    );
    let c1 = registry
        .resolve(&crate::domain::environment_registry::EnvironmentTarget::Ref("C1".into()))
        .unwrap();
    assert_eq!(c1.origin, EnvironmentOrigin::Restored);
}
