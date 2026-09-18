use std::sync::{Arc, Mutex};

use super::super::dto::EnvironmentLiveness;
use super::super::ports::{EnvironmentProcess, EnvironmentRegistryStore};
use super::{GONE_AT_RESTORE, KILL_INTERRUPTED, RestoreRegistry};
use crate::domain::environment_registry::{
    EnvironmentOrigin, EnvironmentRecord, EnvironmentStatus,
};

#[derive(Default)]
struct FakeStore {
    records: Mutex<Vec<EnvironmentRecord>>,
    next: Mutex<u64>,
    fail_load: bool,
    fail_writes: bool,
    forgotten: Mutex<Vec<String>>,
}

impl EnvironmentRegistryStore for FakeStore {
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
        if self.fail_load {
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

fn record(reference: &str, status: EnvironmentStatus) -> EnvironmentRecord {
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

fn store_with(records: Vec<EnvironmentRecord>) -> Arc<FakeStore> {
    let store = FakeStore::default();
    *store.records.lock().unwrap() = records;
    Arc::new(store)
}

fn process(
    f: impl Fn(&EnvironmentRecord) -> EnvironmentLiveness + Send + Sync + 'static,
) -> Arc<dyn EnvironmentProcess> {
    Arc::new(FakeProcess(Box::new(f)))
}

#[test]
fn live_records_are_restored_without_members_gone_ones_stopped_and_unknown_kept() {
    let store = store_with(vec![
        record("C1", EnvironmentStatus::Running),
        record("C2", EnvironmentStatus::Retained),
        record("C3", EnvironmentStatus::CleanupFailed),
        record("C4", EnvironmentStatus::Stopped),
    ]);
    let process = process(|record| match record.environment_ref.as_str() {
        "C1" => EnvironmentLiveness::Running,
        "C2" => EnvironmentLiveness::Gone,
        "C3" => EnvironmentLiveness::Unknown("script missing".into()),
        other => panic!("stopped records are never inspected: {other}"),
    });
    let (registry, report) = RestoreRegistry::new(store.clone(), process).execute("cli:two");
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
    // The corrected status was journalled back.
    let on_file = store.load().unwrap();
    let c2 = on_file.iter().find(|r| r.environment_ref == "C2").unwrap();
    assert_eq!(c2.status, EnvironmentStatus::Stopped);
}

#[test]
fn a_kill_in_flight_when_its_session_ended_becomes_retryable_cleanup_failed() {
    let store = store_with(vec![record("C1", EnvironmentStatus::Killing)]);
    let process = process(|_| panic!("a killing record is not inspected"));
    let (registry, report) = RestoreRegistry::new(store, process).execute("s");
    assert_eq!(report.restored, ["C1"]);
    let c1 = registry.get("C1").unwrap();
    assert_eq!(c1.status, EnvironmentStatus::CleanupFailed);
    assert_eq!(c1.last_error.as_deref(), Some(KILL_INTERRUPTED));
}

#[test]
fn an_unreadable_store_yields_an_empty_registry_that_still_allocates_through_the_store() {
    let store = Arc::new(FakeStore {
        fail_load: true,
        ..Default::default()
    });
    let process = process(|_| EnvironmentLiveness::Running);
    let (registry, report) = RestoreRegistry::new(store.clone(), process).execute("s");
    assert!(registry.entries().is_empty());
    assert_eq!(report.diagnostics.len(), 1);
    assert!(report.diagnostics[0].contains("corrupt"), "{report:?}");
    assert_eq!(registry.mint_ref(), "C1");
    assert_eq!(*store.next.lock().unwrap(), 1);
}

#[test]
fn the_journal_writes_every_transition_and_forgets_a_rolled_back_create() {
    let store = store_with(vec![]);
    let process = process(|_| EnvironmentLiveness::Running);
    let (registry, _) = RestoreRegistry::new(store.clone(), process).execute("s");
    let reference = registry.mint_ref();
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

#[test]
fn a_failing_store_never_panics_and_the_in_memory_counter_takes_over() {
    let store = Arc::new(FakeStore {
        fail_writes: true,
        ..Default::default()
    });
    let process = process(|_| EnvironmentLiveness::Running);
    let (registry, _) = RestoreRegistry::new(store, process).execute("s");
    assert_eq!(registry.mint_ref(), "C1");
    assert_eq!(registry.mint_ref(), "C2");
    registry.commit(record("C2", EnvironmentStatus::Running));
    let claim = registry.begin_kill("C2").unwrap();
    registry.complete_kill(claim);
    assert!(registry.remove("C2").is_some());
}

#[test]
fn an_unseeded_registry_journals_but_inherits_nothing() {
    let store = store_with(vec![record("C5", EnvironmentStatus::Running)]);
    let process = process(|_| panic!("nothing is inspected"));
    let registry = RestoreRegistry::new(store.clone(), process).unseeded("child");
    assert!(registry.entries().is_empty());
    assert_eq!(registry.session(), "child");
    // Refs still come from the shared store: no collision with C5.
    assert_eq!(registry.mint_ref(), "C6");
    registry.commit(record("C6", EnvironmentStatus::Running));
    assert_eq!(store.load().unwrap().len(), 2);
}

#[test]
fn the_restore_debug_is_opaque_over_its_ports() {
    let store = store_with(vec![]);
    let process = process(|_| EnvironmentLiveness::Running);
    let shown = format!("{:?}", RestoreRegistry::new(store, process));
    assert_eq!(shown, "RestoreRegistry { .. }");
}
