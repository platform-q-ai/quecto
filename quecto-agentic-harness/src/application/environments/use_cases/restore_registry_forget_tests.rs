//! #2134: a `stopped` record whose environment directory is gone from disk
//! has nothing left to collect, so the restore forgets it and its number
//! is free again. Refs therefore restart at C1 once nothing is left.
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::super::dto::{EnvironmentLiveness, StateOnDisk};
use super::super::ports::EnvironmentRegistryStore;
use super::restore_registry_tests::{
    failing_store_with, no_hosted, process_with_disk, record, store_with,
};
use super::{NO_ENVIRONMENT_DIR, RestoreRegistry, STATE_GONE_CONTAINER_RUNNING};
use crate::domain::environment_registry::{EnvironmentRecord, EnvironmentStatus};

/// `record` lays its workspace out as the standard scripts do:
/// `/state/<environment_id>/workspace`.
fn laid_out(reference: &str, status: EnvironmentStatus) -> EnvironmentRecord {
    record(reference, status)
}

fn refs(records: &[EnvironmentRecord]) -> Vec<String> {
    records.iter().map(|r| r.environment_ref.clone()).collect()
}

/// A disk on which only `present` directories exist; every asked directory
/// is kept in `asked`.
fn disk(
    present: &'static [&'static str],
    asked: Arc<Mutex<Vec<PathBuf>>>,
) -> impl Fn(&Path) -> StateOnDisk + Send + Sync + 'static {
    move |dir: &Path| {
        asked.lock().unwrap().push(dir.to_path_buf());
        if present.iter().any(|p| Path::new(p) == dir) {
            StateOnDisk::Present
        } else {
            StateOnDisk::Absent
        }
    }
}

/// For records whose state is present (or unknown): the runtime is never
/// asked about a box that left something behind.
fn never_inspected(record: &EnvironmentRecord) -> EnvironmentLiveness {
    panic!(
        "{} left state behind: never inspected",
        record.environment_ref
    )
}

fn gone(_: &EnvironmentRecord) -> EnvironmentLiveness {
    EnvironmentLiveness::Gone
}

#[test]
fn stopped_records_with_nothing_on_disk_are_forgotten_and_refs_restart_at_c1() {
    let store = store_with(vec![
        laid_out("C4", EnvironmentStatus::Stopped),
        laid_out("C9", EnvironmentStatus::Stopped),
    ]);
    let asked = Arc::new(Mutex::new(vec![]));
    let process = process_with_disk(gone, disk(&[], asked.clone()));
    let (registry, report) =
        RestoreRegistry::new(store.clone(), process, no_hosted()).execute("cli:two");
    assert_eq!(report.forgotten, ["C4", "C9"]);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert!(registry.entries().is_empty());
    assert!(store.load().unwrap().is_empty());
    assert_eq!(
        *asked.lock().unwrap(),
        [
            PathBuf::from("/state/env-C4"),
            PathBuf::from("/state/env-C9")
        ]
    );
    assert_eq!(registry.mint_ref().unwrap(), "C1");
}

#[test]
fn a_stopped_record_whose_state_is_left_or_unknown_is_kept() {
    let store = store_with(vec![
        laid_out("C1", EnvironmentStatus::Stopped),
        laid_out("C2", EnvironmentStatus::Stopped),
    ]);
    let process = process_with_disk(never_inspected, |dir: &Path| match dir.to_str() {
        Some("/state/env-C1") => StateOnDisk::Present,
        _ => StateOnDisk::Unknown("permission denied".into()),
    });
    let (registry, report) =
        RestoreRegistry::new(store.clone(), process, no_hosted()).execute("cli:two");
    assert!(report.forgotten.is_empty());
    assert_eq!(refs(&registry.entries()), ["C1", "C2"]);
    assert_eq!(refs(&store.load().unwrap()), ["C1", "C2"]);
}

#[test]
fn only_stopped_records_are_forgotten() {
    // Every other status is somebody's or still being settled; a record
    // this very restore marks stopped is forgotten by the next one.
    let store = store_with(vec![
        laid_out("C1", EnvironmentStatus::Running),
        laid_out("C2", EnvironmentStatus::Retained),
        laid_out("C3", EnvironmentStatus::Killing),
        laid_out("C4", EnvironmentStatus::CleanupFailed),
    ]);
    let asked = Arc::new(Mutex::new(vec![]));
    let process = process_with_disk(|_| EnvironmentLiveness::Gone, disk(&[], asked.clone()));
    let (registry, report) =
        RestoreRegistry::new(store.clone(), process, no_hosted()).execute("cli:two");
    assert!(report.forgotten.is_empty());
    assert!(
        asked.lock().unwrap().is_empty(),
        "only stopped records are looked for"
    );
    assert_eq!(refs(&registry.entries()), ["C1", "C2", "C3", "C4"]);
    assert_eq!(store.load().unwrap().len(), 4);
}

#[test]
fn a_record_whose_workspace_names_no_environment_dir_is_kept_unasked() {
    // A workspace at `/w`: no ancestor is named after the environment id,
    // so where its state lived cannot be told.
    let mut elsewhere = record("C1", EnvironmentStatus::Stopped);
    elsewhere.workspace_path = PathBuf::from("/w");
    let store = store_with(vec![elsewhere]);
    let asked = Arc::new(Mutex::new(vec![]));
    let process = process_with_disk(never_inspected, disk(&[], asked.clone()));
    let (registry, report) =
        RestoreRegistry::new(store.clone(), process, no_hosted()).execute("cli:two");
    assert!(report.forgotten.is_empty());
    assert!(asked.lock().unwrap().is_empty());
    assert_eq!(refs(&registry.entries()), ["C1"]);
    assert_eq!(
        report.kept_stopped,
        [("C1".to_string(), NO_ENVIRONMENT_DIR.to_string())]
    );
}

#[test]
fn a_stopped_record_whose_container_still_runs_or_cannot_be_asked_is_kept() {
    // The state directory is gone, but the runtime is the other half of
    // "nothing left": a running container, or no answer, keeps the record.
    let store = store_with(vec![
        laid_out("C1", EnvironmentStatus::Stopped),
        laid_out("C2", EnvironmentStatus::Stopped),
    ]);
    let liveness = |record: &EnvironmentRecord| match record.environment_ref.as_str() {
        "C1" => EnvironmentLiveness::Running,
        _ => EnvironmentLiveness::Unknown("inspect timed out".into()),
    };
    let process = process_with_disk(liveness, disk(&[], Arc::default()));
    let (registry, report) =
        RestoreRegistry::new(store.clone(), process, no_hosted()).execute("cli:two");
    assert!(report.forgotten.is_empty());
    assert_eq!(refs(&registry.entries()), ["C1", "C2"]);
    assert_eq!(refs(&store.load().unwrap()), ["C1", "C2"]);
    assert_eq!(
        report.kept_stopped,
        [
            ("C1".to_string(), STATE_GONE_CONTAINER_RUNNING.to_string()),
            ("C2".to_string(), "inspect timed out".to_string()),
        ]
    );
}

#[test]
fn an_observing_restore_writes_nothing_and_says_what_it_would_forget() {
    let store = store_with(vec![laid_out("C4", EnvironmentStatus::Stopped)]);
    let process = process_with_disk(gone, disk(&[], Arc::default()));
    let (registry, report) =
        RestoreRegistry::new(store.clone(), process, no_hosted()).observe("cli:two");
    assert!(report.forgotten.is_empty());
    assert_eq!(refs(&store.load().unwrap()), ["C4"]);
    assert!(
        registry.entries().is_empty(),
        "the preview matches the real run"
    );
    assert_eq!(
        report.diagnostics,
        [
            "C4 would be forgotten (stopped; nothing left on disk); not written: this restore only observes"
        ]
    );
}

#[test]
fn a_record_that_cannot_be_forgotten_is_kept_and_said_so() {
    let store = failing_store_with(vec![laid_out("C4", EnvironmentStatus::Stopped)]);
    let process = process_with_disk(gone, disk(&[], Arc::default()));
    let (registry, report) =
        RestoreRegistry::new(store.clone(), process, no_hosted()).execute("cli:two");
    assert!(report.forgotten.is_empty());
    assert_eq!(refs(&registry.entries()), ["C4"]);
    assert_eq!(
        report.diagnostics,
        ["C4 could not be forgotten in the durable registry: disk full"]
    );
}

#[test]
fn a_record_an_older_build_relabelled_is_restored_to_retained_not_forgotten() {
    let mut relabelled = laid_out("C4", EnvironmentStatus::Stopped);
    relabelled.metadata = serde_json::json!({"retained": "run r1 unfinished"});
    relabelled.last_error = Some(crate::domain::environment_registry::GONE_AT_RESTORE.to_string());
    let store = store_with(vec![relabelled]);
    let process = process_with_disk(never_inspected, disk(&[], Arc::default()));
    let (registry, report) =
        RestoreRegistry::new(store.clone(), process, no_hosted()).execute("cli:two");
    assert!(report.forgotten.is_empty());
    assert_eq!(registry.entries()[0].status, EnvironmentStatus::Retained);
}

#[test]
fn a_restore_inspects_within_its_budget_and_the_next_one_finishes() {
    use super::MAX_RESIDUE_INSPECTS;
    let many: Vec<EnvironmentRecord> = (1..=MAX_RESIDUE_INSPECTS + 2)
        .map(|n| laid_out(&format!("C{n}"), EnvironmentStatus::Stopped))
        .collect();
    let store = store_with(many);
    let restore = || {
        let process = process_with_disk(gone, disk(&[], Arc::default()));
        RestoreRegistry::new(store.clone(), process, no_hosted()).execute("cli:two")
    };
    let (registry, report) = restore();
    assert_eq!(report.forgotten.len(), MAX_RESIDUE_INSPECTS);
    assert!(report.kept_stopped.is_empty(), "deferred, not reported");
    assert_eq!(registry.entries().len(), 2);
    let (registry, report) = restore();
    assert_eq!(report.forgotten.len(), 2);
    assert!(registry.entries().is_empty());
}
