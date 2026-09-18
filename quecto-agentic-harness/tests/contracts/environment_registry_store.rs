//! Contract for the `EnvironmentRegistryStore` port (#2024 S4d), proven on
//! the production file store: refs are allocated monotonically and never
//! reused, across store instances (a restart) and across concurrent
//! allocators (two sessions on one base directory); a record is written
//! whole under its ref and replaced by a later write; members are never
//! stored and every loaded record arrives restored, in ref order; a
//! forget removes exactly its record; a broken document is an error and
//! is never silently replaced.
use std::path::Path;
use std::sync::Arc;

use quecto::application::environments::ports::EnvironmentRegistryStore;
use quecto::composition::environments::build_environment_registry_store;
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
fn refs_are_monotonic_never_reused_and_survive_a_restart() {
    let dir = tempfile::TempDir::new().unwrap();
    let first = port(dir.path());
    assert_eq!(first.allocate_ref().unwrap(), 1);
    assert_eq!(first.allocate_ref().unwrap(), 2);
    first.forget("C2").unwrap();
    let restarted = port(dir.path());
    assert_eq!(
        restarted.allocate_ref().unwrap(),
        3,
        "a forgotten ref is never re-minted"
    );
    restarted
        .record(&record("C40", EnvironmentStatus::Running))
        .unwrap();
    assert_eq!(
        port(dir.path()).allocate_ref().unwrap(),
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
                    .map(|_| store.allocate_ref().unwrap())
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
    store.forget("C1").unwrap();
    let loaded = store.load().unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].environment_ref, "C2");
    store.forget("C9").unwrap();
}

#[test]
fn an_empty_base_dir_loads_nothing_and_a_broken_document_is_an_error_left_in_place() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = port(dir.path());
    assert!(store.load().unwrap().is_empty());
    let path = dir.path().join("environments.json");
    std::fs::write(&path, b"garbage").unwrap();
    assert!(store.load().is_err());
    assert!(store.allocate_ref().is_err());
    assert!(
        store
            .record(&record("C1", EnvironmentStatus::Running))
            .is_err()
    );
    assert!(store.forget("C1").is_err());
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
    store.forget("C1").unwrap();
    assert_eq!(
        store
            .correct(&retained, &EnvironmentStatus::Running)
            .unwrap(),
        CorrectionOutcome::Forgotten
    );
    assert!(store.load().unwrap().is_empty());
}
