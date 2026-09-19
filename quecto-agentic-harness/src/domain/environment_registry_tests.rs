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
fn deferred_reconciliation_applies_only_to_unchanged_restored_status_and_preserves_members() {
    let registry = EnvironmentRegistry::new();
    let mut restored = record("C1", "runtime-1");
    restored.status = EnvironmentStatus::Running;
    registry.restore(vec![restored.clone()]);
    registry.add_member("C1", "joined-during-inspect").unwrap();

    let mut judged = restored.clone();
    judged.status = EnvironmentStatus::Retained;
    assert!(registry.reconcile_restored(EnvironmentStatus::Running, judged));
    let applied = registry.get("C1").unwrap();
    assert_eq!(applied.status, EnvironmentStatus::Retained);
    assert_eq!(applied.members, ["joined-during-inspect"]);

    let mut stale = restored;
    stale.status = EnvironmentStatus::Stopped;
    assert!(!registry.reconcile_restored(EnvironmentStatus::Running, stale));
    assert_eq!(
        registry.get("C1").unwrap().status,
        EnvironmentStatus::Retained
    );
}

#[test]
fn deferred_reconciliation_never_changes_a_created_record() {
    let registry = EnvironmentRegistry::new();
    let created = record("C1", "runtime-1");
    registry.commit(created.clone());
    let mut judged = created;
    judged.status = EnvironmentStatus::Stopped;

    assert!(!registry.reconcile_restored(EnvironmentStatus::Running, judged));
    assert_eq!(
        registry.get("C1").unwrap().status,
        EnvironmentStatus::Running
    );
}

#[test]
fn preserved_is_non_joinable_but_explicitly_killable() {
    let registry = EnvironmentRegistry::new();
    let mut environment = record("C1", "runtime-1");
    environment.status = EnvironmentStatus::Retained;
    registry.commit(environment);

    let claim = registry.begin_stop("C1").unwrap();
    registry.complete_stop(claim);
    let preserved = registry.get("C1").unwrap();
    assert_eq!(preserved.status, EnvironmentStatus::Preserved);
    assert!(preserved.members.is_empty());
    assert_eq!(preserved.status_label(), "preserved");
    assert!(
        registry
            .resolve_joinable(&EnvironmentTarget::Ref("C1".into()))
            .is_err()
    );

    let discard = registry.begin_kill("C1").unwrap();
    registry.complete_kill(discard);
    assert_eq!(
        registry.get("C1").unwrap().status,
        EnvironmentStatus::Stopped
    );
}

#[test]
fn stop_eligibility_is_an_affirmative_retained_allowlist() {
    for status in [
        EnvironmentStatus::Running,
        EnvironmentStatus::Killing,
        EnvironmentStatus::Stopped,
        EnvironmentStatus::CleanupFailed,
        EnvironmentStatus::Preserved,
    ] {
        let registry = EnvironmentRegistry::new();
        let mut environment = record("C1", "runtime-1");
        environment.status = status;
        registry.commit(environment);
        assert!(registry.begin_stop("C1").is_err());
    }
}

#[test]
fn refs_are_monotonic_never_reused_and_scoped_per_registry() {
    let registry = EnvironmentRegistry::new();
    let first = registry.mint_ref().unwrap();
    let second = registry.mint_ref().unwrap();
    assert_eq!(first, "C1");
    assert_eq!(second, "C2");

    // A failed launch consumes its ref: removal never recycles it.
    registry.commit(record(&second, "env-a"));
    registry.remove(&second);
    assert_eq!(registry.mint_ref().unwrap(), "C3");

    // Registries are session-scoped, not process-global.
    let other_session = EnvironmentRegistry::new();
    assert_eq!(other_session.mint_ref().unwrap(), "C1");
}

#[test]
fn commit_get_remove_round_trip() {
    let registry = EnvironmentRegistry::new();
    let env_ref = registry.mint_ref().unwrap();
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
    assert_eq!(registry.mint_ref().unwrap(), "C2");
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
    let env_ref = registry.mint_ref().unwrap();
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
                Ok(*counter)
            }
        }),
        recorded: Arc::new({
            let recorded = recorded.clone();
            move |record: &EnvironmentRecord, expected: Option<&EnvironmentStatus>| {
                recorded.lock().unwrap().push(match expected {
                    None => format!("{}:{}", record.environment_ref, record.status_label()),
                    Some(expected) => format!(
                        "{}:{}?{}",
                        record.environment_ref,
                        record.status_label(),
                        expected_label(expected)
                    ),
                });
                JournalWrite::Written
            }
        }),
        forgotten: Arc::new({
            let forgotten = forgotten.clone();
            move |env_ref: &str| forgotten.lock().unwrap().push(env_ref.to_string())
        }),
        reload: Arc::new(|| Err("environments.json: permission denied".into())),
    };
    (journal, recorded, forgotten)
}

#[test]
fn a_journalled_registry_allocates_through_the_journal_and_reports_every_transition() {
    let (journal, recorded, forgotten) = test_journal();
    let registry = EnvironmentRegistry::with_journal(journal, "cli:one");
    assert!(registry.is_durable());
    assert_eq!(registry.session(), "cli:one");
    assert_eq!(registry.mint_ref().unwrap(), "C11");
    assert_eq!(registry.mint_ref().unwrap(), "C12");
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

fn expected_label(status: &EnvironmentStatus) -> &'static str {
    let mut probe = record("Cx", "probe");
    probe.status = status.clone();
    probe.status_label()
}

/// Review F9 (#2033): a durable registry whose journal cannot allocate
/// refuses to mint — never a counter that could collide with a ref a live
/// session holds in the base directory's registry.
#[test]
fn a_journal_that_cannot_allocate_refuses_the_mint() {
    let (mut journal, _, _) = test_journal();
    journal.allocate_ref = Arc::new(|| Err("registry unreadable".into()));
    let registry = EnvironmentRegistry::with_journal(journal, "s");
    registry.restore(vec![record("C7", "env-seven")]);
    let refused = registry.mint_ref().unwrap_err();
    assert_eq!(
        refused,
        RefAllocationError::JournalUnavailable("registry unreadable".into())
    );
    assert!(refused.to_string().contains("registry unreadable"));
    // A journal-less registry still counts in memory.
    let registry = EnvironmentRegistry::new();
    assert_eq!(registry.mint_ref().unwrap(), "C1");
    // A journal answering below the counter never moves it backwards.
    let (mut journal, _, _) = test_journal();
    journal.allocate_ref = Arc::new(|| Ok(1));
    let registry = EnvironmentRegistry::with_journal(journal, "s");
    registry.commit(record("C3", "env-three"));
    assert_eq!(registry.mint_ref().unwrap(), "C4");
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
    assert!(
        recorded.lock().unwrap().is_empty(),
        "seeding is never journalled"
    );
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

/// Review F5 (#2033): a restored record is journalled compare-and-set on
/// the status the journal is known to hold — what was loaded, then what
/// this registry last wrote, or what it learnt superseded it — while a
/// record this session created is written as it is.
#[test]
fn restored_records_are_journalled_compare_and_set_on_the_last_known_status() {
    let (mut journal, recorded, _) = test_journal();
    let superseded = Arc::new(Mutex::new(false));
    journal.recorded = Arc::new({
        let recorded = recorded.clone();
        let superseded = superseded.clone();
        move |record: &EnvironmentRecord, expected: Option<&EnvironmentStatus>| {
            recorded.lock().unwrap().push(format!(
                "{}:{}?{}",
                record.environment_ref,
                record.status_label(),
                expected.map(expected_label).unwrap_or("-")
            ));
            if *superseded.lock().unwrap() {
                JournalWrite::Superseded {
                    current: EnvironmentStatus::Retained,
                    metadata: serde_json::json!({"retained": "elsewhere"}),
                }
            } else {
                JournalWrite::Written
            }
        }
    });
    let registry = EnvironmentRegistry::with_journal(journal, "joiner");
    registry.restore(vec![record("C2", "env-two")]);
    registry.commit(record("C9", "env-nine"));
    registry.add_member("C2", "observer").unwrap();
    // An inspect on the restored record expects what was loaded.
    let claim = registry.begin_inspect("C2", "observer").unwrap();
    registry.record_inspect_success(claim, serde_json::json!({}));
    // The creator moved it on meanwhile: this write is superseded, and
    // the next one expects what the creator wrote.
    *superseded.lock().unwrap() = true;
    let claim = registry.begin_inspect("C2", "observer-2").unwrap();
    registry.record_inspect_failure(claim, "late");
    *superseded.lock().unwrap() = false;
    // Round 2 F-A (#2033): what superseded the write is adopted in memory
    // too, so a later non-transition write carries the file's status —
    // never the stale `running` back onto a `retained` record — and the
    // listing shows what stands on file.
    assert_eq!(
        registry.get("C2").unwrap().status,
        EnvironmentStatus::Retained,
        "the superseding status is adopted in memory"
    );
    assert_eq!(
        registry.get("C2").unwrap().metadata["retained"],
        "elsewhere",
        "the file's metadata is adopted with it (round 3, #2033)"
    );
    let claim = registry.begin_inspect("C2", "observer-3").unwrap();
    registry.record_inspect_failure(claim, "later");
    let claim = registry.begin_kill("C2").unwrap();
    registry.complete_kill(claim);
    assert_eq!(
        recorded.lock().unwrap().as_slice(),
        [
            "C9:empty?-",
            "C2:running?empty",
            "C2:running?empty",
            "C2:retained?retained",
            "C2:killing?retained",
            "C2:stopped?killing",
        ]
    );
}

/// Round 2 F-A (#2033): while this session holds the kill claim, a
/// superseded write does not overwrite `killing` in memory — the claim is
/// the authority until `complete_kill`/`fail_kill` settle it, and the
/// settling write expects what superseded the claim's write.
#[test]
fn a_superseded_write_never_overwrites_an_outstanding_kill_claim() {
    for settle_by_failure in [false, true] {
        let (mut journal, recorded, _) = test_journal();
        let superseded = Arc::new(Mutex::new(false));
        journal.recorded = Arc::new({
            let recorded = recorded.clone();
            let superseded = superseded.clone();
            move |record: &EnvironmentRecord, expected: Option<&EnvironmentStatus>| {
                recorded.lock().unwrap().push(format!(
                    "{}:{}?{}",
                    record.environment_ref,
                    record.status_label(),
                    expected.map(expected_label).unwrap_or("-")
                ));
                if *superseded.lock().unwrap() {
                    JournalWrite::Superseded {
                        current: EnvironmentStatus::Stopped,
                        metadata: serde_json::Value::Null,
                    }
                } else {
                    JournalWrite::Written
                }
            }
        });
        let registry = EnvironmentRegistry::with_journal(journal, "joiner");
        registry.restore(vec![record("C2", "env-two")]);
        *superseded.lock().unwrap() = true;
        let claim = registry.begin_kill("C2").unwrap();
        assert_eq!(
            registry.get("C2").unwrap().status,
            EnvironmentStatus::Killing,
            "the claim holder's killing stands while the claim is outstanding"
        );
        *superseded.lock().unwrap() = false;
        if settle_by_failure {
            registry.fail_kill(claim, "boom");
            assert_eq!(
                registry.get("C2").unwrap().status,
                EnvironmentStatus::CleanupFailed
            );
            assert_eq!(
                recorded.lock().unwrap().as_slice(),
                ["C2:killing?empty", "C2:cleanup-failed?stopped"]
            );
        } else {
            registry.complete_kill(claim);
            assert_eq!(
                registry.get("C2").unwrap().status,
                EnvironmentStatus::Stopped
            );
            assert_eq!(
                recorded.lock().unwrap().as_slice(),
                ["C2:killing?empty", "C2:stopped?stopped"]
            );
        }
        // The claim is settled: a later superseded write adopts again.
        *superseded.lock().unwrap() = true;
        let claim = registry.begin_inspect("C2", "late").unwrap();
        registry.record_inspect_failure(claim, "late");
        assert_eq!(
            registry.get("C2").unwrap().status,
            EnvironmentStatus::Stopped
        );
    }
}

/// Round 2 F-B (#2033): a registry whose durable store could not be read
/// says so on every lookup that would otherwise answer `unknown` — nothing
/// is known, which is not the same as nothing existing — and carries the
/// error for the listing.
#[test]
fn an_unreadable_registry_answers_lookups_with_the_read_error_not_unknown() {
    let (journal, _, _) = test_journal();
    let registry =
        EnvironmentRegistry::unreadable(journal, "s", "environments.json: permission denied");
    assert_eq!(
        registry.read_error().as_deref(),
        Some("environments.json: permission denied")
    );
    let by_ref = registry
        .resolve(&EnvironmentTarget::Ref("C3".into()))
        .unwrap_err();
    assert_eq!(
        by_ref,
        EnvironmentLookupError::Unreadable("environments.json: permission denied".into())
    );
    assert_eq!(
        by_ref.to_string(),
        "registry unreadable: environments.json: permission denied"
    );
    assert!(matches!(
        registry
            .resolve_joinable(&EnvironmentTarget::Name("box".into()))
            .unwrap_err(),
        EnvironmentLookupError::Unreadable(_)
    ));
    // What this session itself committed still resolves.
    registry.commit(record("C1", "env-one"));
    assert_eq!(
        registry
            .resolve(&EnvironmentTarget::Ref("C1".into()))
            .unwrap()
            .environment_id,
        "env-one"
    );
    // A readable registry keeps answering unknown.
    let readable = EnvironmentRegistry::new();
    assert_eq!(readable.read_error(), None);
    assert_eq!(
        readable
            .resolve(&EnvironmentTarget::Ref("C3".into()))
            .unwrap_err(),
        EnvironmentLookupError::Unknown("C3".into())
    );
}

/// Round 3 L2 (#2033): a registry whose store could not be read at
/// startup retries the read on its next lookup. While the store stays
/// unreadable the error is refreshed with its current account; once it
/// reads, what it holds is seeded (what this session created stands as
/// it is) and the error clears — the listing stops reporting it.
#[test]
fn an_unreadable_registry_reads_the_store_again_on_a_lookup_and_clears_the_error_once_it_reads() {
    let (mut journal, _, _) = test_journal();
    let readable = Arc::new(Mutex::new(Err("still corrupt".to_string())));
    let reloads = Arc::new(Mutex::new(0usize));
    journal.reload = Arc::new({
        let readable = readable.clone();
        let reloads = reloads.clone();
        move || {
            *reloads.lock().unwrap() += 1;
            readable.lock().unwrap().clone()
        }
    });
    let registry = EnvironmentRegistry::unreadable(journal, "s", "corrupt at startup");
    registry.commit(record("C5", "env-mine"));
    // Still unreadable: the error is the store's current account.
    assert_eq!(registry.read_error().as_deref(), Some("still corrupt"));
    assert_eq!(
        registry
            .resolve(&EnvironmentTarget::Ref("C1".into()))
            .unwrap_err(),
        EnvironmentLookupError::Unreadable("still corrupt".into())
    );
    assert_eq!(*reloads.lock().unwrap(), 2, "every lookup retries");
    // Repaired in place: the next lookup seeds it.
    let mut on_file = record("C1", "env-theirs");
    on_file.members = vec!["their-member".into()];
    let mut stale_mine = record("C5", "env-mine-on-file");
    stale_mine.status = EnvironmentStatus::Stopped;
    *readable.lock().unwrap() = Ok(vec![on_file, stale_mine]);
    let listed: Vec<String> = registry
        .entries()
        .into_iter()
        .map(|r| format!("{}:{}", r.environment_ref, r.environment_id))
        .collect();
    assert_eq!(
        listed,
        ["C1:env-theirs", "C5:env-mine"],
        "mine stands as it is"
    );
    assert_eq!(registry.read_error(), None);
    let c1 = registry
        .resolve(&EnvironmentTarget::Ref("C1".into()))
        .unwrap();
    assert_eq!(c1.origin, EnvironmentOrigin::Restored);
    assert!(
        c1.members.is_empty(),
        "a seeded record arrives without members"
    );
    assert_eq!(
        registry
            .resolve(&EnvironmentTarget::Ref("C9".into()))
            .unwrap_err(),
        EnvironmentLookupError::Unknown("C9".into()),
        "a miss is a miss again"
    );
    // Recovered once: no further reloads.
    let before = *reloads.lock().unwrap();
    registry.entries();
    registry.read_error();
    assert_eq!(
        *reloads.lock().unwrap(),
        before,
        "a readable registry never reloads"
    );
    // A journal-less registry never reloads either.
    assert_eq!(EnvironmentRegistry::new().read_error(), None);
}

/// Round 3 (#2033, cosmetic): adopting a superseding write merges the
/// file's metadata under this session's own keys; a non-object on either
/// side never turns an object into nothing.
#[test]
fn merge_metadata_keeps_both_sides_keys_and_lets_the_writer_win() {
    use super::merge_metadata;
    assert_eq!(
        merge_metadata(
            serde_json::json!({"a": 1, "retained": "why"}),
            &serde_json::json!({"a": 2, "cause": "seen"})
        ),
        serde_json::json!({"a": 2, "retained": "why", "cause": "seen"})
    );
    assert_eq!(
        merge_metadata(serde_json::json!({"a": 1}), &serde_json::Value::Null),
        serde_json::json!({"a": 1})
    );
    assert_eq!(
        merge_metadata(serde_json::Value::Null, &serde_json::json!({"a": 1})),
        serde_json::json!({"a": 1})
    );
    assert_eq!(
        merge_metadata(serde_json::json!({"a": 1}), &serde_json::json!("text")),
        serde_json::json!("text")
    );
}
