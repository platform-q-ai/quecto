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

#[test]
fn the_store_names_its_document_under_the_base_dir() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = FileEnvironmentRegistryStore::for_base_dir(dir.path());
    assert_eq!(store.path(), dir.path().join(REGISTRY_FILE_NAME));
    assert!(format!("{store:?}").contains("environments.json"));
    // An unreadable document (a directory in its place) is an error
    // naming the path, not an empty registry.
    std::fs::create_dir_all(store.path()).unwrap();
    let error = store.load().unwrap_err();
    assert!(error.contains("environments.json"), "{error}");
}

/// Review F5 (#2033): a joiner's journal writes on a *restored* record are
/// compare-and-set on the status it last loaded or wrote, never a whole
/// record replacement — so a joiner's inspect racing the creator's
/// retention leaves the creator's `Retained` on file, and a joiner's
/// later kill (a real transition from what is now on file) still lands.
#[test]
fn a_joiners_write_on_a_restored_record_never_reverts_the_creators_status() {
    use crate::application::environments::dto::EnvironmentLiveness;
    use crate::application::environments::ports::EnvironmentProcess;
    use crate::application::environments::use_cases::RestoreRegistry;
    use std::sync::Arc;

    struct AlwaysRunning;
    impl EnvironmentProcess for AlwaysRunning {
        fn observe(&self, _: &EnvironmentRecord) -> EnvironmentLiveness {
            EnvironmentLiveness::Running
        }
        fn cleanup(&self, _: &EnvironmentRecord) -> Result<(), String> {
            Ok(())
        }
    }
    let dir = tempfile::TempDir::new().unwrap();
    let store: Arc<dyn EnvironmentRegistryStore> =
        Arc::new(FileEnvironmentRegistryStore::for_base_dir(dir.path()));
    let restore = RestoreRegistry::new(
        store.clone(),
        Arc::new(AlwaysRunning),
        Arc::new(crate::infrastructure::tools::environment_commands::HostedStoreObservation),
    );
    // The creator's session: its own record, one member.
    let creator = restore.unseeded("creator");
    let mut created = record("C1", EnvironmentStatus::Running);
    created.members.clear();
    creator.commit(created);
    creator.add_member("C1", "member-1").unwrap();
    // The joiner's session restores C1 (running) and joins it.
    let (joiner, report) = restore.execute("joiner");
    assert_eq!(report.restored, ["C1"]);
    joiner.add_member("C1", "observer").unwrap();
    let inspect = joiner.begin_inspect("C1", "observer").expect("claim");
    // Meanwhile the creator's last member leaves and the run retains it.
    let claim = creator.remove_member("C1", "member-1").unwrap().unwrap();
    creator.retain(claim, "run ended: complete");
    assert_eq!(store.load().unwrap()[0].status, EnvironmentStatus::Retained);
    // The joiner's inspect lands after: metadata only, status untouched.
    joiner.record_inspect_success(inspect, serde_json::json!({"cause": "observed"}));
    let on_file = &store.load().unwrap()[0];
    assert_eq!(
        on_file.status,
        EnvironmentStatus::Retained,
        "the joiner's inspect must not revert the creator's retention: {on_file:?}"
    );
    assert_eq!(on_file.metadata["retained"], "run ended: complete");
    // Round 3 (#2033, cosmetic): the joiner adopts the creator's metadata
    // with the status it learnt superseded its write, and keeps its own.
    let in_memory = joiner.get("C1").unwrap();
    assert_eq!(in_memory.status, EnvironmentStatus::Retained);
    assert_eq!(in_memory.metadata["retained"], "run ended: complete");
    assert_eq!(in_memory.metadata["cause"], "observed");
    // A joiner's explicit kill is a transition from what is on file now,
    // and its write merges over the file's metadata: the creator's
    // `retained` reason survives a write that never named it.
    let claim = joiner.begin_kill("C1").unwrap();
    let on_file = &store.load().unwrap()[0];
    assert_eq!(on_file.status, EnvironmentStatus::Killing);
    assert_eq!(on_file.metadata["retained"], "run ended: complete");
    assert_eq!(on_file.metadata["cause"], "observed");
    joiner.complete_kill(claim);
    assert_eq!(store.load().unwrap()[0].status, EnvironmentStatus::Stopped);
}

/// Round 3 (#2033, cosmetic): a conditional write merges its metadata
/// over what is on file — a key the writer never saw (another session's)
/// survives, a key it names is its value — while an unconditional write
/// (the creator's own record) replaces the record whole.
#[test]
fn a_correction_merges_its_metadata_over_the_files_and_a_record_replaces_it() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = FileEnvironmentRegistryStore::for_base_dir(dir.path());
    let mut created = record("C1", EnvironmentStatus::Running);
    created.metadata = serde_json::json!({"container": "quecto-1", "retained": "ended"});
    store.record(&created).unwrap();
    let mut joiner = created.clone();
    joiner.metadata = serde_json::json!({"container": "quecto-1", "cause": "observed"});
    joiner.status = EnvironmentStatus::Killing;
    assert_eq!(
        store.correct(&joiner, &EnvironmentStatus::Running).unwrap(),
        crate::application::environments::dto::CorrectionOutcome::Applied
    );
    let on_file = &store.load().unwrap()[0];
    assert_eq!(on_file.status, EnvironmentStatus::Killing);
    assert_eq!(
        on_file.metadata,
        serde_json::json!({"container": "quecto-1", "retained": "ended", "cause": "observed"})
    );
    // A superseded correction hands back the file's record, metadata and all.
    let superseded = store.correct(&joiner, &EnvironmentStatus::Running).unwrap();
    match superseded {
        crate::application::environments::dto::CorrectionOutcome::Superseded(current) => {
            assert_eq!(current.metadata["retained"], "ended");
        }
        other => panic!("{other:?}"),
    }
    // An unconditional write replaces the record whole.
    let mut rewritten = created.clone();
    rewritten.metadata = serde_json::json!({"container": "quecto-1"});
    store.record(&rewritten).unwrap();
    assert_eq!(
        store.load().unwrap()[0].metadata,
        serde_json::json!({"container": "quecto-1"})
    );
}
