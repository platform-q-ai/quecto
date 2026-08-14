//! #1460 session-key single-writer ownership contract.
//!
//! Target behavior: acquiring a key takes an exclusive OS lock on a pid
//! stamp sidecar; a second claim on a key whose lock is held by another
//! live process is refused with an error naming the key and stamped owner;
//! a stamp left by a dead process carries no lock and is reclaimed;
//! dropping the guard releases the claim.

use super::*;

/// Simulate another live process owning `key`: an independently opened file
/// description holding the exclusive lock (flock semantics are per open
/// description, exactly as another process would hold it), with `owner_pid`
/// stamped for diagnostics.
fn hold_as_foreign_owner(dir: &Path, key: &str, owner_pid: u32) -> std::fs::File {
    use std::io::Write;
    let file = open_stamp_file(dir, key).expect("open stamp file");
    file.try_lock().expect("foreign owner lock");
    let mut writer = &file;
    file.set_len(0).expect("truncate stamp");
    writer
        .write_all(owner_pid.to_string().as_bytes())
        .expect("write foreign owner pid");
    file
}

#[test]
fn acquire_writes_owner_stamp_with_pid() {
    let dir = tempfile::tempdir().expect("tempdir");
    let guard = SessionOwnershipGuard::acquire(dir.path(), "stamped-key")
        .expect("first acquire must succeed");
    assert!(
        guard.stamp_path().exists(),
        "acquiring a session key must write an ownership stamp sidecar"
    );
    let contents = std::fs::read_to_string(guard.stamp_path()).expect("read stamp");
    assert!(
        contents.contains(&std::process::id().to_string()),
        "ownership stamp must record the owning pid, got: {contents:?}"
    );
}

#[test]
fn second_acquire_of_live_owned_key_is_refused_with_clear_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Owner and claimant must be DIFFERENT pids in the message, or an error
    // that echoes the claimant instead of reading the stamp would pass. The
    // parent (test runner) is a live process that is never this one.
    let owner_pid = std::os::unix::process::parent_id();
    assert_ne!(owner_pid, std::process::id());
    let _held = hold_as_foreign_owner(dir.path(), "shared-key", owner_pid);

    let second = SessionOwnershipGuard::acquire(dir.path(), "shared-key");
    let err = match second {
        Err(e) => e.to_string(),
        Ok(_) => panic!("second acquire of a key owned by a live process must be refused"),
    };
    assert!(
        err.contains("shared-key"),
        "refusal must name the session key, got: {err}"
    );
    assert!(
        err.contains(&owner_pid.to_string()),
        "refusal must name the owning pid (not the claimant), got: {err}"
    );
}

#[test]
fn stamp_left_by_dead_process_is_reclaimed() {
    // A dead process's lock is released by the kernel; only its stamp file
    // (with its stale pid) remains, and that carries no ownership.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut child = std::process::Command::new("true")
        .spawn()
        .expect("spawn true");
    let dead = child.id();
    child.wait().expect("wait true");
    let stamp = ownership_stamp_path(dir.path(), "stale-key");
    std::fs::write(&stamp, dead.to_string()).expect("write stale stamp");

    let guard = SessionOwnershipGuard::acquire(dir.path(), "stale-key")
        .expect("a stamp left by a dead process must be reclaimable");
    let contents = std::fs::read_to_string(guard.stamp_path()).expect("read stamp");
    assert!(
        contents.contains(&std::process::id().to_string()),
        "reclaiming must rewrite the stamp with the new owner pid, got: {contents:?}"
    );
}

#[test]
fn unparseable_stamp_is_reclaimed_not_permanently_stuck() {
    // A corrupt unlocked stamp must never strand a key forever: with no lock
    // held there is no live owner to protect, so the claim reclaims it.
    let dir = tempfile::tempdir().expect("tempdir");
    let stamp = ownership_stamp_path(dir.path(), "corrupt-key");
    std::fs::write(&stamp, "not-a-pid").expect("write corrupt stamp");

    let guard = SessionOwnershipGuard::acquire(dir.path(), "corrupt-key")
        .expect("an unreadable stamp must be reclaimable");
    let contents = std::fs::read_to_string(guard.stamp_path()).expect("read stamp");
    assert!(
        contents.contains(&std::process::id().to_string()),
        "reclaiming must rewrite the stamp with the new owner pid, got: {contents:?}"
    );
}

#[test]
fn dropping_the_guard_releases_ownership() {
    let dir = tempfile::tempdir().expect("tempdir");
    let guard = SessionOwnershipGuard::acquire(dir.path(), "released-key")
        .expect("first acquire must succeed");
    drop(guard);
    // The stamp file may remain (never unlinked, to avoid the orphaned-inode
    // double-owner race), but the key must be reacquirable.
    SessionOwnershipGuard::acquire(dir.path(), "released-key")
        .expect("a released key must be reacquirable");
}

#[test]
fn acquire_reports_stamp_open_failure() {
    let dir = tempfile::tempdir().expect("tempdir");
    let blocked_dir = dir.path().join("blocked-dir");
    std::fs::create_dir(&blocked_dir).expect("create blocked dir");
    let blocker = ownership_stamp_path(&blocked_dir, "blocked-key");
    std::fs::create_dir(&blocker).expect("directory at stamp path makes stamp open fail");
    let err = SessionOwnershipGuard::acquire(&blocked_dir, "blocked-key")
        .expect_err("a directory at the stamp path must be reported");
    assert!(
        err.to_string()
            .contains("failed to open ownership stamp for 'blocked-key'"),
        "{err}"
    );
}

#[test]
fn registry_claim_is_idempotent_and_release_relinquishes_key() {
    let dir = tempfile::tempdir().expect("tempdir");
    let registry = SessionOwnershipRegistry::default();
    registry
        .claim(dir.path(), "registry-key")
        .expect("first registry claim must acquire the key");
    registry
        .claim(dir.path(), "registry-key")
        .expect("repeat claim by the same registry must be idempotent");

    let blocked = SessionOwnershipGuard::acquire(dir.path(), "registry-key")
        .expect_err("registry must hold the OS lock until explicit release");
    assert!(blocked.to_string().contains("registry-key"), "{blocked}");

    registry.release("registry-key");
    SessionOwnershipGuard::acquire(dir.path(), "registry-key")
        .expect("release must relinquish the OS lock for the key");
}

#[test]
fn concurrent_acquires_yield_exactly_one_owner() {
    // The atomic try_lock means two simultaneous claimants can never both
    // win — the race the old read-remove-recreate reclaim allowed.
    let dir = tempfile::tempdir().expect("tempdir");
    let dir_path = dir.path().to_path_buf();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let dir_path = dir_path.clone();
            let barrier = std::sync::Arc::clone(&barrier);
            std::thread::spawn(move || {
                // Independent open file descriptions racing on one key.
                let file = open_stamp_file(&dir_path, "raced-key").expect("open stamp");
                barrier.wait();
                let won = file.try_lock().is_ok();
                // Hold the lock until every claimant has attempted.
                barrier.wait();
                won
            })
        })
        .collect();
    let winners = handles
        .into_iter()
        .map(|h| h.join())
        .filter(|r| matches!(r, Ok(true)))
        .count();
    assert_eq!(
        winners, 1,
        "exactly one of N concurrent claimants may win the key"
    );
}

#[test]
fn owner_description_never_claims_a_dead_process_is_live() {
    // The refusal message used to assert "live process" for whatever pid the
    // stamp happened to contain, including our own and long-dead ones.
    assert_eq!(
        describe_owner(Some(std::process::id())),
        format!("this process ({})", std::process::id())
    );
    assert_eq!(describe_owner(None), "an unidentified process");

    // A pid that cannot be running: pid 0 is never a userspace process here.
    let dead = describe_owner(Some(0));
    assert!(
        dead.contains("no longer running"),
        "a dead pid must not be reported as live: {dead}"
    );
}

#[test]
fn release_then_reclaim_succeeds_while_children_are_being_spawned() {
    // Regression: a concurrent fork duplicates the lock descriptor, so a
    // reclaim straight after release could transiently see the key as held.
    let dir = tempfile::tempdir().expect("tempdir");
    let spawning = std::thread::spawn(|| {
        for _ in 0..40 {
            let _ = std::process::Command::new("true").status();
        }
    });
    for _ in 0..15 {
        let registry = SessionOwnershipRegistry::default();
        registry
            .claim(dir.path(), "churn-key")
            .expect("claim must acquire the key");
        registry.release("churn-key");
        SessionOwnershipGuard::acquire(dir.path(), "churn-key")
            .expect("release must relinquish the key even while forks are in flight");
    }
    spawning.join().expect("spawner thread");
}
