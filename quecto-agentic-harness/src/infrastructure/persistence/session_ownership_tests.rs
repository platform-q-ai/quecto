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
fn owner_description_is_honest_and_actionable() {
    // The refusal used to assert "live process" for whatever pid the stamp held
    // — including our own and long-dead ones — and told the user to close a
    // process that was not running.
    let ours = describe_owner(Some(std::process::id()));
    assert!(
        ours.contains("this process already holds it"),
        "our own pid must not read as a foreign owner: {ours}"
    );
    assert!(
        !ours.contains("close that agent"),
        "must not advise closing our own agent: {ours}"
    );

    // pid 0 is never a userspace process: kill(0, ..) targets our own process
    // group and would otherwise report success.
    let dead = describe_owner(Some(0));
    assert!(
        dead.contains("not running"),
        "a dead pid must not be reported as live: {dead}"
    );
    assert!(
        !dead.contains("close that agent"),
        "must not advise closing a process that is not running: {dead}"
    );

    // A live foreign owner is the one case where the advice is actionable, and
    // the pid must appear (the refusal contract the BDD steps assert on).
    let live = describe_owner(Some(other_live_pid()));
    assert!(
        live.contains(&other_live_pid().to_string()) && live.contains("close that agent"),
        "a live owner must be named and actionable: {live}"
    );

    assert!(describe_owner(None).contains("unidentified"));
}

#[test]
fn a_process_owned_by_another_user_counts_as_live() {
    // kill(pid, 0) fails with EPERM for a process this user may not signal. It
    // exists, so treating any failure as "gone" would report a live owner as
    // dead — the shared sessions dir / uid-mapped container case.
    assert!(kill_probe_means_live(0, None), "signalled successfully");
    assert!(
        kill_probe_means_live(-1, Some(libc::EPERM)),
        "EPERM means the process exists but is not ours to signal"
    );
    assert!(
        !kill_probe_means_live(-1, Some(libc::ESRCH)),
        "ESRCH is the only answer that means the process is gone"
    );
}

/// A pid that is certainly alive and is not this process: our parent.
fn other_live_pid() -> u32 {
    // SAFETY: getppid takes no arguments and cannot fail.
    unsafe { libc::getppid() as u32 }
}

#[test]
fn release_unlocks_even_when_a_descriptor_duplicate_survives() {
    use std::os::fd::AsRawFd;

    // A lock belongs to the open file description, so a child forked anywhere
    // in this process keeps it alive until it execs. `dup` reproduces that
    // deterministically: closing our descriptor alone leaves the lock held,
    // and only an explicit unlock releases it for the duplicate too.
    let dir = tempfile::tempdir().expect("tempdir");
    let registry = SessionOwnershipRegistry::default();
    registry
        .claim(dir.path(), "dup-key")
        .expect("claim must acquire the key");

    let duplicate = {
        let owned = registry.owned.lock().expect("registry mutex");
        let guard = owned.get("dup-key").expect("claimed guard");
        // SAFETY: dup on a descriptor owned by the live guard.
        unsafe { libc::dup(guard._lock_file.as_raw_fd()) }
    };
    assert!(duplicate >= 0, "dup must succeed");

    registry.release("dup-key");
    let reclaimed = SessionOwnershipGuard::acquire(dir.path(), "dup-key");

    // SAFETY: closing the duplicate we created above.
    unsafe { libc::close(duplicate) };
    reclaimed.expect("release must unlock for duplicated descriptors too");
}
