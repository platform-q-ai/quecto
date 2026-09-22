//! Contract for the `EnvironmentRegistryStore` port (#2024 S4d), proven on
//! the production file store: a ref is one above everything that still
//! exists — a record, a mint younger than a create can take, the caller's
//! floor — and is reserved on file from the moment it is minted, across
//! store instances (a restart) and across concurrent allocators (two
//! sessions on one base directory), so that once everything is collected
//! the next ref is `1` again (#2070); a ref names one environment: a
//! record, correction or forget for another environment under it lands
//! nothing; a record is written whole under its ref and replaced by a
//! later write of the same environment; members are never stored and
//! every loaded record arrives restored, in ref order; a broken document
//! is an error and is never silently replaced.
use std::path::Path;
use std::sync::Arc;

use quecto::application::environments::ports::EnvironmentRegistryStore;
use quecto::composition::environments::{
    build_environment_registry_store, build_environment_registry_store_at,
};
use quecto::domain::environment_registry::{
    EnvironmentOrigin, EnvironmentRecord, EnvironmentStatus,
};

fn port(base_dir: &Path) -> Arc<dyn EnvironmentRegistryStore> {
    build_environment_registry_store(base_dir)
}

fn record(reference: &str, status: EnvironmentStatus) -> EnvironmentRecord {
    EnvironmentRecord {
        environment_ref: reference.to_string(),
        environment_id: format!("env-{reference}"),
        environment_uuid: format!("uuid-{reference}"),
        name: Some(format!("named-{reference}")),
        workspace_path: format!("/state/env-{reference}/workspace").into(),
        repository: "https://example.test/repo".into(),
        script_name: "official".into(),
        retained_exec_argv: vec!["exec".into(), "--x".into()],
        retained_kill_argv: vec!["kill".into()],
        retained_cleanup_argv: vec!["cleanup".into()],
        retained_inspect_argv: vec!["inspect".into()],
        members: vec!["member-of-the-creating-session".into()],
        status,
        metadata: serde_json::json!({"container": format!("quecto-env-{reference}"), "checkout": "/x"}),
        last_error: Some("last".into()),
        origin: EnvironmentOrigin::Created,
        created_by: "cli:creator".into(),
        created_at: Some(42),
    }
}

#[test]
fn a_minted_ref_stays_reserved_across_a_restart_and_never_falls_below_a_record() {
    let dir = tempfile::TempDir::new().unwrap();
    let first = port(dir.path());
    assert_eq!(first.allocate_ref(0).unwrap(), 1);
    assert_eq!(first.allocate_ref(0).unwrap(), 2);
    let restarted = port(dir.path());
    assert_eq!(
        restarted.allocate_ref(0).unwrap(),
        3,
        "both mints are still in flight: neither is re-minted"
    );
    restarted
        .record(&record("C40", EnvironmentStatus::Running))
        .unwrap();
    assert_eq!(
        port(dir.path()).allocate_ref(0).unwrap(),
        41,
        "never below a recorded ref"
    );
}

#[test]
fn concurrent_allocators_never_share_a_ref() {
    let dir = tempfile::TempDir::new().unwrap();
    let base = dir.path().to_path_buf();
    let handles: Vec<_> = (0..6)
        .map(|_| {
            let base = base.clone();
            std::thread::spawn(move || {
                let store = port(&base);
                (0..8)
                    .map(|_| store.allocate_ref(0).unwrap())
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    let mut all: Vec<u64> = handles
        .into_iter()
        .flat_map(|h| h.join().unwrap())
        .collect();
    all.sort_unstable();
    assert_eq!(all, (1..=48).collect::<Vec<_>>());
}

#[test]
fn records_are_stored_whole_without_members_and_loaded_restored_in_ref_order() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = port(dir.path());
    store
        .record(&record("C12", EnvironmentStatus::Retained))
        .unwrap();
    store
        .record(&record("C3", EnvironmentStatus::CleanupFailed))
        .unwrap();
    let loaded = port(dir.path()).load().unwrap();
    assert_eq!(
        loaded
            .iter()
            .map(|r| r.environment_ref.as_str())
            .collect::<Vec<_>>(),
        ["C3", "C12"]
    );
    let mut expected = record("C12", EnvironmentStatus::Retained);
    expected.members.clear();
    expected.origin = EnvironmentOrigin::Restored;
    assert_eq!(loaded[1], expected);
}

#[test]
fn a_later_record_replaces_the_earlier_and_forget_removes_only_its_ref() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = port(dir.path());
    store
        .record(&record("C1", EnvironmentStatus::Running))
        .unwrap();
    store
        .record(&record("C2", EnvironmentStatus::Running))
        .unwrap();
    let mut stopped = record("C1", EnvironmentStatus::Stopped);
    stopped.last_error = None;
    store.record(&stopped).unwrap();
    let loaded = store.load().unwrap();
    assert_eq!(loaded[0].status, EnvironmentStatus::Stopped);
    assert_eq!(loaded[0].last_error, None);
    store
        .forget(&record("C1", EnvironmentStatus::Stopped))
        .unwrap();
    let loaded = store.load().unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].environment_ref, "C2");
    store
        .forget(&record("C9", EnvironmentStatus::Stopped))
        .unwrap();
}

#[test]
fn an_empty_base_dir_loads_nothing_and_a_broken_document_is_an_error_left_in_place() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = port(dir.path());
    assert!(store.load().unwrap().is_empty());
    let path = dir.path().join("environments.json");
    std::fs::write(&path, b"garbage").unwrap();
    assert!(store.load().is_err());
    assert!(store.allocate_ref(0).is_err());
    assert!(
        store
            .record(&record("C1", EnvironmentStatus::Running))
            .is_err()
    );
    assert!(
        store
            .forget(&record("C1", EnvironmentStatus::Stopped))
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"garbage");
}

#[test]
fn a_correction_is_written_only_while_the_record_still_has_the_expected_status() {
    use quecto::application::environments::dto::CorrectionOutcome;
    let dir = tempfile::TempDir::new().unwrap();
    let store = port(dir.path());
    store
        .record(&record("C1", EnvironmentStatus::Running))
        .unwrap();
    let mut stopped = record("C1", EnvironmentStatus::Stopped);
    stopped.last_error = Some("gone at restore".into());
    assert_eq!(
        store
            .correct(&stopped, &EnvironmentStatus::Running)
            .unwrap(),
        CorrectionOutcome::Applied
    );
    assert_eq!(store.load().unwrap()[0].status, EnvironmentStatus::Stopped);
    // Somebody else moved on: the correction is refused and what is on
    // file comes back.
    let mut retained = record("C1", EnvironmentStatus::Retained);
    retained.members.clear();
    match store
        .correct(&stopped, &EnvironmentStatus::Running)
        .unwrap()
    {
        CorrectionOutcome::Superseded(current) => {
            assert_eq!(current.status, EnvironmentStatus::Stopped);
            assert_eq!(current.last_error.as_deref(), Some("gone at restore"));
        }
        other => panic!("{other:?}"),
    }
    store
        .forget(&record("C1", EnvironmentStatus::Stopped))
        .unwrap();
    assert_eq!(
        store
            .correct(&retained, &EnvironmentStatus::Running)
            .unwrap(),
        CorrectionOutcome::Forgotten
    );
    assert!(store.load().unwrap().is_empty());
}

/// The clock the store reads, so a test can age a mint past its grace.
fn clocked(
    base_dir: &Path,
    now: &Arc<std::sync::atomic::AtomicU64>,
) -> Arc<dyn EnvironmentRegistryStore> {
    let now = Arc::clone(now);
    build_environment_registry_store_at(base_dir, move || {
        now.load(std::sync::atomic::Ordering::SeqCst)
    })
}

/// #2070: the number is one above everything that still exists — so once
/// every record is forgotten and every mint recorded, released or older
/// than a create can take, the next ref is `1` again.
#[test]
fn refs_restart_at_one_once_nothing_recorded_or_in_flight_remains() {
    let dir = tempfile::TempDir::new().unwrap();
    let now = Arc::new(std::sync::atomic::AtomicU64::new(1_700_000_000));
    let store = clocked(dir.path(), &now);
    assert_eq!(store.allocate_ref(0).unwrap(), 1);
    store
        .record(&record("C1", EnvironmentStatus::Running))
        .unwrap();
    assert_eq!(store.allocate_ref(0).unwrap(), 2);
    store
        .record(&record("C2", EnvironmentStatus::Stopped))
        .unwrap();
    store
        .forget(&record("C1", EnvironmentStatus::Running))
        .unwrap();
    assert_eq!(store.allocate_ref(0).unwrap(), 3, "C2 still exists");
    store.release_ref(3).unwrap();
    store
        .forget(&record("C2", EnvironmentStatus::Stopped))
        .unwrap();
    assert_eq!(store.allocate_ref(0).unwrap(), 1, "everything collected");
    // That mint is in flight: a concurrent session (a second instance) is
    // handed the number above it ...
    assert_eq!(clocked(dir.path(), &now).allocate_ref(0).unwrap(), 2);
    // ... until both are older than a create can take.
    now.fetch_add(60 * 60 + 1, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(clocked(dir.path(), &now).allocate_ref(0).unwrap(), 1);
}

/// #2070: the floor is what the caller still holds that the file may have
/// forgotten; the answer is above it and reserved on file like any mint.
#[test]
fn the_floor_lifts_the_answer_and_the_answer_is_reserved_for_a_second_instance() {
    let dir = tempfile::TempDir::new().unwrap();
    assert_eq!(port(dir.path()).allocate_ref(4).unwrap(), 4);
    assert_eq!(
        port(dir.path()).allocate_ref(0).unwrap(),
        5,
        "4 is in flight"
    );
    assert_eq!(
        port(dir.path()).allocate_ref(2).unwrap(),
        6,
        "a floor below the file's own next changes nothing"
    );
}

/// #2070: a ref names one environment. Once another environment is on
/// file under it, this environment's record, correction and forget land
/// nothing — the other session's record stands as it wrote it.
#[test]
fn a_ref_taken_by_another_environment_refuses_a_record_and_a_correction_and_survives_a_forget() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = port(dir.path());
    store
        .record(&record("C1", EnvironmentStatus::Running))
        .unwrap();
    let mut stale = record("C1", EnvironmentStatus::Stopped);
    stale.environment_uuid = "uuid-stale".into();
    stale.environment_id = "env-stale".into();
    let refused = store.record(&stale).unwrap_err();
    assert!(
        refused.contains("already records environment uuid-C1"),
        "{refused}"
    );
    let refused = store
        .correct(&stale, &EnvironmentStatus::Running)
        .unwrap_err();
    assert!(
        refused.contains("now records environment uuid-C1"),
        "{refused}"
    );
    store.forget(&stale).unwrap();
    let on_file = port(dir.path()).load().unwrap();
    assert_eq!(on_file.len(), 1);
    assert_eq!(on_file[0].environment_uuid, "uuid-C1");
    assert_eq!(on_file[0].status, EnvironmentStatus::Running);
}
