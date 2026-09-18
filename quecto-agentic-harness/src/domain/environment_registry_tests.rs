use super::*;

fn record(env_ref: &str, id: &str) -> EnvironmentRecord {
    EnvironmentRecord {
        environment_ref: env_ref.to_string(),
        environment_id: id.to_string(),
        environment_uuid: mint_environment_uuid(),
        name: None,
        workspace_path: PathBuf::from("/workspace"),
        repository: "https://example.invalid/repo.git".to_string(),
        script_name: "default".to_string(),
        retained_exec_argv: vec![],
        retained_kill_argv: vec![],
        retained_cleanup_argv: vec![],
        retained_inspect_argv: vec![],
        members: vec![],
        status: EnvironmentStatus::Running,
        metadata: serde_json::json!({}),
        last_error: None,
        origin: crate::domain::environment_registry::EnvironmentOrigin::Created,
        created_by: String::new(),
        created_at: None,
    }
}

#[test]
fn refs_are_monotonic_never_reused_and_scoped_per_registry() {
    let registry = EnvironmentRegistry::new();
    let first = registry.mint_ref();
    let second = registry.mint_ref();
    assert_eq!(first, "C1");
    assert_eq!(second, "C2");

    // A failed launch consumes its ref: removal never recycles it.
    registry.commit(record(&second, "env-a"));
    registry.remove(&second);
    assert_eq!(registry.mint_ref(), "C3");

    // Registries are session-scoped, not process-global.
    let other_session = EnvironmentRegistry::new();
    assert_eq!(other_session.mint_ref(), "C1");
}

#[test]
fn commit_get_remove_round_trip() {
    let registry = EnvironmentRegistry::new();
    let env_ref = registry.mint_ref();
    registry.commit(record(&env_ref, "env-a"));
    assert_eq!(registry.get(&env_ref).unwrap().environment_id, "env-a");
    assert_eq!(registry.entries().len(), 1);

    assert_eq!(registry.remove(&env_ref).unwrap().environment_id, "env-a");
    assert!(registry.get(&env_ref).is_none());
    assert!(registry.entries().is_empty());
    assert!(registry.remove(&env_ref).is_none());
}

#[test]
fn lock_poison_recovery_keeps_registry_usable() {
    let registry = EnvironmentRegistry::new();
    registry.commit(record("C1", "env-a"));

    let poisoner = registry.clone();
    let _ = std::thread::spawn(move || {
        let _guard = poisoner.state.lock().unwrap();
        panic!("poison the environment registry lock");
    })
    .join();

    // Every accessor must recover from the poisoned lock without losing state.
    // The committed C1 advanced the counter (#2024 S4d: a seeded ref is
    // never re-minted), so the next mint is C2.
    assert_eq!(registry.mint_ref(), "C2");
    registry.commit(record("C2", "env-b"));
    assert_eq!(registry.get("C1").unwrap().environment_id, "env-a");
    assert_eq!(registry.entries().len(), 2);
    assert_eq!(registry.remove("C2").unwrap().environment_id, "env-b");
}

/// The membership/kill/inspect mutators must also recover from a poisoned
/// lock (their `unwrap_or_else(into_inner)` closures are production paths).
#[test]
fn lock_poison_recovery_covers_member_kill_and_inspect_paths() {
    let registry = EnvironmentRegistry::new();
    let env_ref = registry.mint_ref();
    registry.commit(record(&env_ref, "env-a"));

    let poisoner = registry.clone();
    let _ = std::thread::spawn(move || {
        let _guard = poisoner.state.lock().unwrap();
        panic!("poison the environment registry lock");
    })
    .join();

    registry.add_member(&env_ref, "member-a").unwrap();
    registry.add_member(&env_ref, "member-b").unwrap();
    assert!(
        registry
            .remove_member(&env_ref, "member-a")
            .unwrap()
            .is_none()
    );

    let claim = registry.begin_inspect(&env_ref, "member-b").expect("claim");
    registry.record_inspect_failure(claim, "inspect broke");
    assert!(registry.get(&env_ref).unwrap().last_error.is_some());
    let claim = registry.begin_inspect(&env_ref, "member-c").expect("claim");
    registry.record_inspect_success(claim, serde_json::json!({"cause": "ok"}));
    assert!(registry.get(&env_ref).unwrap().last_error.is_none());

    let claim = registry.begin_kill(&env_ref).unwrap();
    registry.complete_kill(claim);
    assert_eq!(
        registry.get(&env_ref).unwrap().status,
        EnvironmentStatus::Stopped
    );
}

// ─── Durable registry (#2024 S4d) ────────────────────────────────────────────

type Seen = Arc<Mutex<Vec<String>>>;

fn test_journal() -> (EnvironmentJournal, Seen, Seen) {
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let forgotten = Arc::new(Mutex::new(Vec::new()));
    let counter = Arc::new(Mutex::new(10u64));
    let journal = EnvironmentJournal {
        allocate_ref: Arc::new({
            let counter = counter.clone();
            move || {
                let mut counter = counter.lock().unwrap();
                *counter += 1;
                Some(*counter)
            }
        }),
        recorded: Arc::new({
            let recorded = recorded.clone();
            move |record: &EnvironmentRecord| {
                recorded.lock().unwrap().push(format!(
                    "{}:{}",
                    record.environment_ref,
                    record.status_label()
                ))
            }
        }),
        forgotten: Arc::new({
            let forgotten = forgotten.clone();
            move |env_ref: &str| forgotten.lock().unwrap().push(env_ref.to_string())
        }),
    };
    (journal, recorded, forgotten)
}

#[test]
fn a_journalled_registry_allocates_through_the_journal_and_reports_every_transition() {
    let (journal, recorded, forgotten) = test_journal();
    let registry = EnvironmentRegistry::with_journal(journal, "cli:one");
    assert!(registry.is_durable());
    assert_eq!(registry.session(), "cli:one");
    assert_eq!(registry.mint_ref(), "C11");
    assert_eq!(registry.mint_ref(), "C12");
    registry.commit(record("C12", "env-a"));
    registry.add_member("C12", "m1").unwrap();
    let claim = registry.remove_member("C12", "m1").unwrap().unwrap();
    registry.retain(claim, "run ended");
    let claim = registry.begin_kill("C12").unwrap();
    registry.fail_kill(claim, "boom");
    let inspect = registry.begin_inspect("C12", "m1").unwrap();
    registry.record_inspect_failure(inspect, "no inspect");
    let claim = registry.begin_kill("C12").unwrap();
    registry.complete_kill(claim);
    registry.remove("C12");
    assert_eq!(
        recorded.lock().unwrap().as_slice(),
        [
            "C12:empty",
            "C12:killing",
            "C12:retained",
            "C12:killing",
            "C12:cleanup-failed",
            "C12:cleanup-failed",
            "C12:killing",
            "C12:stopped",
        ]
    );
    assert_eq!(forgotten.lock().unwrap().as_slice(), ["C12"]);
    assert!(format!("{registry:?}").contains("EnvironmentJournal"));
}

#[test]
fn a_journal_that_cannot_allocate_falls_back_to_the_counter_past_every_seen_ref() {
    let (mut journal, _, _) = test_journal();
    journal.allocate_ref = Arc::new(|| None);
    let registry = EnvironmentRegistry::with_journal(journal, "s");
    registry.restore(vec![record("C7", "env-seven")]);
    assert_eq!(registry.mint_ref(), "C8");
    // A journal answering below the counter never moves it backwards.
    let (mut journal, _, _) = test_journal();
    journal.allocate_ref = Arc::new(|| Some(1));
    let registry = EnvironmentRegistry::with_journal(journal, "s");
    registry.commit(record("C3", "env-three"));
    assert_eq!(registry.mint_ref(), "C4");
}

#[test]
fn restored_records_arrive_without_members_and_are_never_torn_down_by_a_joiner() {
    let (journal, recorded, _) = test_journal();
    let registry = EnvironmentRegistry::with_journal(journal, "s");
    let mut seeded = record("C2", "env-two");
    seeded.members = vec!["stale".into()];
    registry.restore(vec![seeded]);
    let restored = registry.get("C2").unwrap();
    assert_eq!(restored.origin, EnvironmentOrigin::Restored);
    assert!(restored.members.is_empty());
    assert_eq!(recorded.lock().unwrap().as_slice(), ["C2:empty"]);
    registry.add_member("C2", "joiner").unwrap();
    assert!(
        registry.remove_member("C2", "joiner").unwrap().is_none(),
        "a joiner leaving a restored environment claims no kill"
    );
    assert_eq!(
        registry.get("C2").unwrap().status,
        EnvironmentStatus::Running
    );
    // An explicit kill still ends it.
    let claim = registry.begin_kill("C2").unwrap();
    registry.complete_kill(claim);
    assert_eq!(
        registry.get("C2").unwrap().status,
        EnvironmentStatus::Stopped
    );
}

#[test]
fn ref_numbers_parse_only_well_formed_refs() {
    assert_eq!(ref_number("C12"), Some(12));
    assert_eq!(ref_number("C"), None);
    assert_eq!(ref_number("C1a"), None);
    assert_eq!(ref_number("D1"), None);
    assert_eq!(ref_number(""), None);
}
