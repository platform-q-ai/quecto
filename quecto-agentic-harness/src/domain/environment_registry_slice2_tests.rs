//! Slice 2 (#1369): the session environment registry becomes authoritative for
//! environment UUID, name, retained script set, members, status, and errors.

use super::*;

fn commit_env(reg: &EnvironmentRegistry, name: Option<&str>) -> String {
    let env_ref = reg.mint_ref();
    reg.commit(EnvironmentRecord {
        environment_ref: env_ref.clone(),
        environment_id: format!("runtime-{env_ref}"),
        environment_uuid: format!("uuid-{env_ref}"),
        name: name.map(str::to_string),
        workspace_path: std::path::PathBuf::from(format!("/ws/{env_ref}")),
        repository: "https://example.invalid/repo.git".to_string(),
        script_name: "default".to_string(),
        retained_exec_argv: vec!["exec.sh".to_string()],
        retained_kill_argv: vec!["kill.sh".to_string()],
        retained_cleanup_argv: vec!["cleanup.sh".to_string()],
        members: vec![],
        status: EnvironmentStatus::Running,
        metadata: serde_json::json!({}),
        last_error: None,
    });
    env_ref
}

#[test]
fn minted_environment_uuids_are_fresh_and_distinct_from_other_identities() {
    // Production commit paths mint the hidden environment UUID with this
    // function (see the spawn adapter), so distinctness is pinned at the mint.
    let first = mint_environment_uuid();
    let second = mint_environment_uuid();
    assert!(!first.is_empty());
    assert_ne!(first, second, "each environment gets a fresh UUID");
    assert!(
        !first.starts_with('C'),
        "environment UUIDs are not CN refs: {first}"
    );
}

#[test]
fn resolve_by_ref_returns_the_committed_record() {
    let reg = EnvironmentRegistry::new();
    let env_ref = commit_env(&reg, None);
    let rec = reg
        .resolve(&EnvironmentTarget::Ref(env_ref.clone()))
        .unwrap();
    assert_eq!(rec.environment_ref, env_ref);
}

#[test]
fn resolve_unknown_ref_fails_without_guessing() {
    let reg = EnvironmentRegistry::new();
    commit_env(&reg, None);
    let err = reg
        .resolve(&EnvironmentTarget::Ref("C9".to_string()))
        .unwrap_err();
    assert!(matches!(err, EnvironmentLookupError::Unknown(_)), "{err:?}");
}

#[test]
fn resolve_by_unique_name_succeeds() {
    let reg = EnvironmentRegistry::new();
    let env_ref = commit_env(&reg, Some("review-env"));
    commit_env(&reg, Some("other-env"));
    let rec = reg
        .resolve(&EnvironmentTarget::Name("review-env".to_string()))
        .unwrap();
    assert_eq!(rec.environment_ref, env_ref);
}

#[test]
fn resolve_ambiguous_name_fails_without_guessing() {
    let reg = EnvironmentRegistry::new();
    commit_env(&reg, Some("dup-env"));
    commit_env(&reg, Some("dup-env"));
    let err = reg
        .resolve(&EnvironmentTarget::Name("dup-env".to_string()))
        .unwrap_err();
    assert!(
        matches!(err, EnvironmentLookupError::Ambiguous(_)),
        "{err:?}"
    );
}

#[test]
fn resolve_stopped_environment_for_join_fails() {
    let reg = EnvironmentRegistry::new();
    let env_ref = commit_env(&reg, None);
    let claim = reg.begin_kill(&env_ref).unwrap();
    reg.complete_kill(claim);
    let err = reg
        .resolve_joinable(&EnvironmentTarget::Ref(env_ref.clone()))
        .unwrap_err();
    assert!(matches!(err, EnvironmentLookupError::Stopped(_)), "{err:?}");
}

#[test]
fn stopped_environments_stay_listed_and_refs_are_never_reused() {
    let reg = EnvironmentRegistry::new();
    let first = commit_env(&reg, None);
    let claim = reg.begin_kill(&first).unwrap();
    reg.complete_kill(claim);
    // The stopped record stays visible to get_containers.
    assert_eq!(reg.get(&first).unwrap().status, EnvironmentStatus::Stopped);
    // The next environment gets a fresh ref.
    let second = commit_env(&reg, None);
    assert_ne!(first, second);
}

#[test]
fn members_are_recorded_in_join_order_without_duplicates() {
    let reg = EnvironmentRegistry::new();
    let env_ref = commit_env(&reg, None);
    reg.add_member(&env_ref, "agent-uuid-a").unwrap();
    reg.add_member(&env_ref, "agent-uuid-b").unwrap();
    reg.add_member(&env_ref, "agent-uuid-a").unwrap();
    let rec = reg.get(&env_ref).unwrap();
    assert_eq!(rec.members, vec!["agent-uuid-a", "agent-uuid-b"]);
}

#[test]
fn resolve_cleanup_failed_environment_for_join_fails_as_stale() {
    let reg = EnvironmentRegistry::new();
    let env_ref = commit_env(&reg, None);
    let claim = reg.begin_kill(&env_ref).unwrap();
    reg.fail_kill(claim, "kill.sh exited 1");
    let err = reg
        .resolve_joinable(&EnvironmentTarget::Ref(env_ref.clone()))
        .unwrap_err();
    assert!(matches!(err, EnvironmentLookupError::Stale(_)), "{err:?}");
}

#[test]
fn resolve_unknown_name_fails_without_guessing() {
    let reg = EnvironmentRegistry::new();
    commit_env(&reg, Some("review-env"));
    let err = reg
        .resolve(&EnvironmentTarget::Name("missing-env".to_string()))
        .unwrap_err();
    assert!(matches!(err, EnvironmentLookupError::Unknown(_)), "{err:?}");
}

#[test]
fn status_labels_distinguish_running_empty_stopped_and_failed() {
    let reg = EnvironmentRegistry::new();
    let env_ref = commit_env(&reg, None);
    // Live with zero members reads as empty; with members it is running.
    assert_eq!(reg.get(&env_ref).unwrap().status_label(), "empty");
    reg.add_member(&env_ref, "a").unwrap();
    assert_eq!(reg.get(&env_ref).unwrap().status_label(), "running");
    let failed = commit_env(&reg, None);
    let claim = reg.begin_kill(&failed).unwrap();
    assert_eq!(reg.get(&failed).unwrap().status_label(), "killing");
    reg.fail_kill(claim, "boom");
    assert_eq!(reg.get(&failed).unwrap().status_label(), "cleanup-failed");
    let claim = reg.begin_kill(&failed).unwrap();
    reg.complete_kill(claim);
    assert_eq!(reg.get(&failed).unwrap().status_label(), "stopped");
}

#[test]
fn removing_a_non_final_member_does_not_claim_cleanup() {
    let reg = EnvironmentRegistry::new();
    let env_ref = commit_env(&reg, None);
    reg.add_member(&env_ref, "a").unwrap();
    reg.add_member(&env_ref, "b").unwrap();
    let removal = reg.remove_member(&env_ref, "a").unwrap();
    assert!(removal.is_none());
    assert_eq!(
        reg.get(&env_ref).unwrap().status,
        EnvironmentStatus::Running
    );
}

#[test]
fn final_member_removal_claims_cleanup_exactly_once() {
    let reg = EnvironmentRegistry::new();
    let env_ref = commit_env(&reg, None);
    reg.add_member(&env_ref, "a").unwrap();
    let removal = reg.remove_member(&env_ref, "a").unwrap();
    assert!(removal.is_some());
    // A duplicate/racing removal of the same member cannot claim again.
    let removal = reg.remove_member(&env_ref, "a").unwrap();
    assert!(removal.is_none());
}

#[test]
fn concurrent_final_member_exits_yield_exactly_one_cleanup_claim() {
    let reg = EnvironmentRegistry::new();
    let env_ref = commit_env(&reg, None);
    reg.add_member(&env_ref, "a").unwrap();
    let reg_a = reg.clone();
    let reg_b = reg.clone();
    let ra = env_ref.clone();
    let rb = env_ref.clone();
    let ta = std::thread::spawn(move || reg_a.remove_member(&ra, "a").unwrap());
    let tb = std::thread::spawn(move || reg_b.remove_member(&rb, "a").unwrap());
    let claims = [ta.join().unwrap(), tb.join().unwrap()]
        .iter()
        .filter(|r| r.is_some())
        .count();
    assert_eq!(claims, 1, "exactly one racer may claim final cleanup");
}

#[test]
fn begin_kill_is_exclusive_until_completed_or_failed() {
    let reg = EnvironmentRegistry::new();
    let env_ref = commit_env(&reg, None);
    let claim = reg.begin_kill(&env_ref).unwrap();
    // A second claim while one is outstanding must be refused (no double-kill).
    assert!(reg.begin_kill(&env_ref).is_err());
    reg.complete_kill(claim);
    // After a successful kill the environment is stopped: no further claims.
    assert!(reg.begin_kill(&env_ref).is_err());
}

#[test]
fn kill_failure_persists_retryable_cleanup_failed_state() {
    let reg = EnvironmentRegistry::new();
    let env_ref = commit_env(&reg, None);
    let claim = reg.begin_kill(&env_ref).unwrap();
    reg.fail_kill(claim, "kill.sh exited 1");
    let rec = reg.get(&env_ref).unwrap();
    assert_eq!(rec.status, EnvironmentStatus::CleanupFailed);
    assert_eq!(rec.last_error.as_deref(), Some("kill.sh exited 1"));
    // Retry is allowed after failure and can commit stopped.
    let claim = reg.begin_kill(&env_ref).unwrap();
    reg.complete_kill(claim);
    assert_eq!(
        reg.get(&env_ref).unwrap().status,
        EnvironmentStatus::Stopped
    );
}

#[test]
fn stopped_is_committed_only_after_kill_success() {
    let reg = EnvironmentRegistry::new();
    let env_ref = commit_env(&reg, None);
    let claim = reg.begin_kill(&env_ref).unwrap();
    // While the kill is in flight the environment must not read as stopped.
    assert_ne!(
        reg.get(&env_ref).unwrap().status,
        EnvironmentStatus::Stopped
    );
    reg.complete_kill(claim);
    assert_eq!(
        reg.get(&env_ref).unwrap().status,
        EnvironmentStatus::Stopped
    );
}

#[test]
fn retained_argv_round_trips_through_commit() {
    // Plain round-trip pin; the behavioral "retained script set survives a
    // config default change" acceptance is exercised end-to-end in the slice-2
    // BDD feature with two distinguishable script sets.
    let reg = EnvironmentRegistry::new();
    let env_ref = commit_env(&reg, None);
    let rec = reg.get(&env_ref).unwrap();
    assert_eq!(rec.retained_exec_argv, vec!["exec.sh"]);
    assert_eq!(rec.retained_kill_argv, vec!["kill.sh"]);
}

#[test]
fn lock_poison_recovery_keeps_slice2_operations_usable() {
    let reg = EnvironmentRegistry::new();
    let env_ref = commit_env(&reg, Some("poison-env"));
    reg.add_member(&env_ref, "member-a").unwrap();

    let poisoner = reg.clone();
    let _ = std::thread::spawn(move || {
        let _guard = poisoner.state.lock().unwrap();
        panic!("poison the environment registry lock");
    })
    .join();

    // Every slice-2 accessor must recover from the poisoned lock.
    assert!(
        reg.resolve(&EnvironmentTarget::Ref(env_ref.clone()))
            .is_ok()
    );
    assert!(
        reg.resolve_joinable(&EnvironmentTarget::Name("poison-env".to_string()))
            .is_ok()
    );
    reg.add_member(&env_ref, "member-b").unwrap();
    let removal = reg.remove_member(&env_ref, "member-b").unwrap();
    assert!(removal.is_none());
    let claim = reg.begin_kill(&env_ref).unwrap();
    reg.fail_kill(claim, "poisoned-kill");
    let claim = reg.begin_kill(&env_ref).unwrap();
    reg.complete_kill(claim);
    assert_eq!(
        reg.get(&env_ref).unwrap().status,
        EnvironmentStatus::Stopped
    );
}

#[test]
fn name_resolution_ignores_stopped_records_so_names_are_reusable() {
    let reg = EnvironmentRegistry::new();
    let first = commit_env(&reg, Some("review-env"));
    let claim = reg.begin_kill(&first).unwrap();
    reg.complete_kill(claim);
    // A fresh environment reusing the name resolves uniquely: the stopped
    // record must not make the name ambiguous for the rest of the session.
    let second = commit_env(&reg, Some("review-env"));
    let rec = reg
        .resolve(&EnvironmentTarget::Name("review-env".to_string()))
        .unwrap();
    assert_eq!(rec.environment_ref, second);
}

#[test]
fn name_matching_only_stopped_records_fails_as_stopped_not_unknown() {
    let reg = EnvironmentRegistry::new();
    let env_ref = commit_env(&reg, Some("done-env"));
    let claim = reg.begin_kill(&env_ref).unwrap();
    reg.complete_kill(claim);
    let err = reg
        .resolve(&EnvironmentTarget::Name("done-env".to_string()))
        .unwrap_err();
    assert!(matches!(err, EnvironmentLookupError::Stopped(_)), "{err:?}");
}

#[test]
fn add_member_is_refused_once_the_environment_is_no_longer_running() {
    let reg = EnvironmentRegistry::new();
    let stopped = commit_env(&reg, None);
    let claim = reg.begin_kill(&stopped).unwrap();
    // While the kill claim is outstanding a join must fail as stale.
    let err = reg.add_member(&stopped, "late-joiner").unwrap_err();
    assert!(matches!(err, EnvironmentLookupError::Stale(_)), "{err:?}");
    reg.complete_kill(claim);
    let err = reg.add_member(&stopped, "late-joiner").unwrap_err();
    assert!(matches!(err, EnvironmentLookupError::Stopped(_)), "{err:?}");
    assert!(reg.get(&stopped).unwrap().members.is_empty());

    let failed = commit_env(&reg, None);
    let claim = reg.begin_kill(&failed).unwrap();
    reg.fail_kill(claim, "boom");
    let err = reg.add_member(&failed, "late-joiner").unwrap_err();
    assert!(matches!(err, EnvironmentLookupError::Stale(_)), "{err:?}");
}

#[test]
fn retained_cleanup_argv_round_trips_through_commit() {
    let reg = EnvironmentRegistry::new();
    let env_ref = commit_env(&reg, None);
    assert_eq!(
        reg.get(&env_ref).unwrap().retained_cleanup_argv,
        vec!["cleanup.sh"]
    );
}

#[test]
fn lookup_errors_render_actionable_messages() {
    assert!(
        EnvironmentLookupError::Unknown("C9".into())
            .to_string()
            .contains("unknown")
    );
    assert!(
        EnvironmentLookupError::Ambiguous("dup".into())
            .to_string()
            .contains("ambiguous")
    );
    assert!(
        EnvironmentLookupError::Stopped("C1".into())
            .to_string()
            .contains("stopped")
    );
    assert!(
        EnvironmentLookupError::Stale("C1".into())
            .to_string()
            .contains("retry kill_container")
    );
}
