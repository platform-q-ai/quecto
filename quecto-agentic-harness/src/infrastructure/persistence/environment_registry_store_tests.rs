use super::*;
use crate::application::environments::ports::EnvironmentRegistryStore;
use crate::domain::environment_registry::EnvironmentOrigin;

fn record(reference: &str, status: EnvironmentStatus) -> EnvironmentRecord {
    EnvironmentRecord {
        environment_ref: reference.to_string(),
        environment_id: format!("env-{reference}"),
        environment_uuid: format!("uuid-{reference}"),
        name: Some(format!("named-{reference}")),
        workspace_path: PathBuf::from(format!("/state/env-{reference}/workspace")),
        repository: "https://example.test/repo".into(),
        script_name: "official".into(),
        retained_exec_argv: vec!["exec".into()],
        retained_kill_argv: vec!["kill".into()],
        retained_cleanup_argv: vec!["cleanup".into()],
        retained_inspect_argv: vec!["inspect".into()],
        members: vec!["member-1".into()],
        status,
        metadata: serde_json::json!({"container": format!("quecto-env-{reference}")}),
        last_error: None,
        origin: EnvironmentOrigin::Created,
        created_by: "cli:one".into(),
        created_at: Some(1_700_000_000),
    }
}

#[test]
fn allocates_monotonic_refs_across_store_instances_and_never_below_a_recorded_ref() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = FileEnvironmentRegistryStore::for_base_dir(dir.path());
    assert_eq!(store.allocate_ref().unwrap(), 1);
    assert_eq!(store.allocate_ref().unwrap(), 2);
    // A record beyond the counter (an older document) pulls it up.
    store
        .record(&record("C9", EnvironmentStatus::Running))
        .unwrap();
    let again = FileEnvironmentRegistryStore::for_base_dir(dir.path());
    assert_eq!(again.allocate_ref().unwrap(), 10);
    assert_eq!(store.allocate_ref().unwrap(), 11);
}

#[test]
fn records_round_trip_without_members_and_arrive_restored_in_ref_order() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = FileEnvironmentRegistryStore::for_base_dir(dir.path());
    store
        .record(&record("C10", EnvironmentStatus::Retained))
        .unwrap();
    store
        .record(&record("C2", EnvironmentStatus::Stopped))
        .unwrap();
    let loaded = store.load().unwrap();
    let refs: Vec<&str> = loaded.iter().map(|r| r.environment_ref.as_str()).collect();
    assert_eq!(refs, ["C2", "C10"]);
    let restored = &loaded[1];
    let mut expected = record("C10", EnvironmentStatus::Retained);
    expected.members.clear();
    expected.origin = EnvironmentOrigin::Restored;
    assert_eq!(restored, &expected);
    // The file is private and holds no member.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(store.path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
    let text = std::fs::read_to_string(store.path()).unwrap();
    assert!(!text.contains("member-1"), "{text}");
    assert!(text.contains("\"created_by\": \"cli:one\""), "{text}");
}

#[test]
fn a_record_replaces_its_predecessor_and_forget_removes_it() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = FileEnvironmentRegistryStore::for_base_dir(dir.path());
    store
        .record(&record("C1", EnvironmentStatus::Running))
        .unwrap();
    let mut stopped = record("C1", EnvironmentStatus::Stopped);
    stopped.last_error = Some("gone".into());
    store.record(&stopped).unwrap();
    let loaded = store.load().unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].status, EnvironmentStatus::Stopped);
    assert_eq!(loaded[0].last_error.as_deref(), Some("gone"));
    store.forget("C1").unwrap();
    assert!(store.load().unwrap().is_empty());
    // Forgetting again is idempotent.
    store.forget("C1").unwrap();
}

#[test]
fn a_missing_document_is_empty_and_a_broken_one_is_an_error_never_replaced() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = FileEnvironmentRegistryStore::for_base_dir(dir.path());
    assert!(store.load().unwrap().is_empty());
    std::fs::write(store.path(), b"{not json").unwrap();
    let error = store.load().unwrap_err();
    assert!(
        error.contains("not a valid environment registry"),
        "{error}"
    );
    let error = store
        .record(&record("C1", EnvironmentStatus::Running))
        .unwrap_err();
    assert!(
        error.contains("not a valid environment registry"),
        "{error}"
    );
    assert_eq!(std::fs::read(store.path()).unwrap(), b"{not json");
    std::fs::write(store.path(), b"{\"version\": 99}").unwrap();
    let error = store.allocate_ref().unwrap_err();
    assert!(error.contains("version 99"), "{error}");
}

#[test]
fn concurrent_allocations_from_many_threads_never_collide() {
    let dir = tempfile::TempDir::new().unwrap();
    let base = dir.path().to_path_buf();
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let base = base.clone();
            std::thread::spawn(move || {
                let store = FileEnvironmentRegistryStore::for_base_dir(&base);
                (0..5)
                    .map(|_| store.allocate_ref().unwrap())
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    let mut all: Vec<u64> = handles
        .into_iter()
        .flat_map(|handle| handle.join().unwrap())
        .collect();
    all.sort_unstable();
    assert_eq!(all, (1..=40).collect::<Vec<_>>());
}
