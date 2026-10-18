//! #2206: only a `stopped` record is claimable for removal; the claim is
//! the exclusive kill claim, and completing it forgets the record in the
//! durable registry too.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::*;

fn stopped(env_ref: &str) -> EnvironmentRecord {
    EnvironmentRecord {
        environment_ref: env_ref.to_string(),
        environment_id: format!("env-{env_ref}"),
        environment_uuid: mint_environment_uuid(),
        name: None,
        workspace_path: PathBuf::from("/workspace"),
        repository: String::new(),
        script_name: "default".to_string(),
        retained_exec_argv: vec![],
        retained_kill_argv: vec!["kill.sh".to_string()],
        retained_cleanup_argv: vec!["cleanup.sh".to_string()],
        retained_inspect_argv: vec![],
        members: vec![],
        status: EnvironmentStatus::Stopped,
        metadata: serde_json::json!({}),
        last_error: None,
        origin: EnvironmentOrigin::Created,
        created_by: String::new(),
        created_at: None,
    }
}

fn journalled() -> (EnvironmentRegistry, Arc<Mutex<Vec<String>>>) {
    let forgotten = Arc::new(Mutex::new(Vec::new()));
    let journal = EnvironmentJournal {
        allocate_ref: Arc::new(|floor| Ok(floor.max(1))),
        release_ref: Arc::new(|_| {}),
        recorded: Arc::new(|_: &EnvironmentRecord, _: Option<&EnvironmentStatus>| {
            JournalWrite::Written
        }),
        forgotten: Arc::new({
            let forgotten = forgotten.clone();
            move |record: &EnvironmentRecord| {
                forgotten
                    .lock()
                    .unwrap()
                    .push(record.environment_ref.clone())
            }
        }),
        reload: Arc::new(|| Ok(Vec::new())),
    };
    (EnvironmentRegistry::with_journal(journal, "cli"), forgotten)
}

#[test]
fn a_stopped_record_is_claimed_then_forgotten_durably() {
    let (registry, forgotten) = journalled();
    registry.commit(stopped("C3"));

    let claim = registry
        .begin_removal("C3")
        .expect("a stopped record is claimable");
    assert_eq!(
        registry.get("C3").unwrap().status,
        EnvironmentStatus::Killing
    );
    assert!(
        registry.begin_removal("C3").is_err(),
        "one removal at a time"
    );
    assert!(
        registry.begin_kill("C3").is_err(),
        "no kill races the removal"
    );

    registry.complete_removal(claim);
    assert!(registry.get("C3").is_none());
    assert_eq!(forgotten.lock().unwrap().as_slice(), ["C3".to_string()]);
}

#[test]
fn only_a_stopped_record_is_claimable_for_removal() {
    let registry = EnvironmentRegistry::new();
    for (env_ref, status) in [
        ("C1", EnvironmentStatus::Running),
        ("C2", EnvironmentStatus::Retained),
        ("C3", EnvironmentStatus::Killing),
        ("C4", EnvironmentStatus::CleanupFailed),
    ] {
        registry.commit(EnvironmentRecord {
            status: status.clone(),
            ..stopped(env_ref)
        });
        assert!(
            matches!(
                registry.begin_removal(env_ref),
                Err(EnvironmentLookupError::Stale(_))
            ),
            "{status:?}"
        );
        assert_eq!(registry.get(env_ref).unwrap().status, status, "untouched");
    }
    assert!(matches!(
        registry.begin_removal("C9"),
        Err(EnvironmentLookupError::Unknown(_))
    ));
}

#[test]
fn a_failed_removal_is_cleanup_failed_and_retryable() {
    let registry = EnvironmentRegistry::new();
    registry.commit(stopped("C5"));
    let claim = registry.begin_removal("C5").unwrap();
    registry.fail_removal(claim, "podman: busy");
    let record = registry.get("C5").unwrap();
    assert_eq!(record.status, EnvironmentStatus::CleanupFailed);
    assert_eq!(record.last_error.as_deref(), Some("podman: busy"));
    assert!(
        registry.begin_removal("C5").is_ok(),
        "the owed removal is retried"
    );
}

#[test]
fn the_removal_claim_is_the_authority_over_a_superseding_write() {
    // A restored record's write is conditional; while this session holds
    // the removal claim, what another session wrote does not replace the
    // claimed status in memory (as for a kill claim), so the removal
    // completes on the record it claimed.
    let journal = EnvironmentJournal {
        allocate_ref: Arc::new(|floor| Ok(floor.max(1))),
        release_ref: Arc::new(|_| {}),
        recorded: Arc::new(|_: &EnvironmentRecord, _: Option<&EnvironmentStatus>| {
            JournalWrite::Superseded {
                current: EnvironmentStatus::Stopped,
                metadata: serde_json::json!({}),
            }
        }),
        forgotten: Arc::new(|_: &EnvironmentRecord| {}),
        reload: Arc::new(|| Ok(Vec::new())),
    };
    let registry = EnvironmentRegistry::with_journal(journal, "cli");
    registry.restore(vec![stopped("C6")]);
    let claim = registry.begin_removal("C6").unwrap();
    assert_eq!(
        registry.get("C6").unwrap().status,
        EnvironmentStatus::Killing,
        "the claim stands"
    );
    registry.complete_removal(claim);
    assert!(registry.get("C6").is_none());
}

#[test]
fn a_failed_removal_is_owed_and_a_released_claim_returns_where_it_was() {
    let registry = EnvironmentRegistry::new();
    registry.commit(stopped("C7"));
    // Refused after its claim: back to stopped, untouched.
    let claim = registry.begin_removal("C7").unwrap();
    registry.release_removal(claim);
    let record = registry.get("C7").unwrap();
    assert_eq!(record.status, EnvironmentStatus::Stopped);
    assert!(!record.removal_pending());
    // A failed cleanup: cleanup-failed and owed; claimable for removal.
    let claim = registry.begin_removal("C7").unwrap();
    registry.fail_removal(claim, "busy");
    let record = registry.get("C7").unwrap();
    assert_eq!(record.status, EnvironmentStatus::CleanupFailed);
    assert!(record.removal_pending() && record.removable());
    let claim = registry
        .begin_removal("C7")
        .expect("the owed removal is retried");
    registry.release_removal(claim);
    assert!(registry.get("C7").unwrap().removal_pending(), "still owed");
    // An ordinary cleanup-failed record is no removal.
    registry.commit(EnvironmentRecord {
        status: EnvironmentStatus::CleanupFailed,
        ..stopped("C8")
    });
    assert!(!registry.get("C8").unwrap().removable());
    assert!(registry.begin_removal("C8").is_err());
    // Metadata that is not an object still carries the mark.
    registry.commit(EnvironmentRecord {
        metadata: serde_json::Value::Null,
        ..stopped("C9")
    });
    let claim = registry.begin_removal("C9").unwrap();
    registry.fail_removal(claim, "busy");
    assert!(registry.get("C9").unwrap().removal_pending());
}

/// #2206 round 4: a released removal returns to the status it was claimed
/// from, whatever the metadata says; claiming a plainly stopped record
/// drops a stale owed mark.
#[test]
fn a_released_removal_restores_exactly_the_claimed_status() {
    let registry = EnvironmentRegistry::new();
    registry.commit(EnvironmentRecord {
        metadata: serde_json::json!({ REMOVAL_PENDING: true }),
        ..stopped("C3")
    });
    let claim = registry.begin_removal("C3").unwrap();
    assert_eq!(claim.claimed_from(), &EnvironmentStatus::Stopped);
    registry.release_removal(claim);
    let record = registry.get("C3").unwrap();
    assert_eq!(record.status, EnvironmentStatus::Stopped);
    assert!(
        record.metadata.get(REMOVAL_PENDING).is_none(),
        "stale mark dropped"
    );
    // An owed removal claimed and released stays owed.
    let claim = registry.begin_removal("C3").unwrap();
    registry.fail_removal(claim, "busy");
    let claim = registry.begin_removal("C3").unwrap();
    assert_eq!(claim.claimed_from(), &EnvironmentStatus::CleanupFailed);
    registry.release_removal(claim);
    assert!(registry.get("C3").unwrap().removal_pending());
}

/// The release reads the claim, never the metadata: a mark that appears
/// while the removal is claimed (merged by an inspect outcome) cannot
/// turn a refused removal of a stopped record into an owed one.
#[test]
fn a_release_ignores_a_mark_that_appeared_under_the_claim() {
    let registry = EnvironmentRegistry::new();
    registry.commit(stopped("C4"));
    let claim = registry.begin_removal("C4").unwrap();
    let inspect = registry.begin_inspect("C4", "someone").unwrap();
    registry.record_inspect_success(inspect, serde_json::json!({ REMOVAL_PENDING: true }));
    registry.release_removal(claim);
    assert_eq!(
        registry.get("C4").unwrap().status,
        EnvironmentStatus::Stopped
    );
}
