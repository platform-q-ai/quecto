//! #1460 session-key single-writer ownership contract (RED).
//!
//! Target behavior: acquiring a key writes a pid stamp sidecar; a second
//! claim on a key whose stamped owner is alive is refused with an error
//! naming the key and owning pid; a stamp left by a dead process is
//! reclaimed; dropping the guard releases the claim.

use super::*;

/// Spawn-and-reap a child so we hold a pid that is guaranteed dead.
fn dead_pid() -> u32 {
    let child = std::process::Command::new("true")
        .spawn()
        .expect("spawn true");
    let pid = child.id();
    let mut child = child;
    child.wait().expect("wait true");
    pid
}

#[test]
fn acquire_writes_owner_stamp_with_pid() {
    let dir = tempfile::tempdir().expect("tempdir");
    let guard = SessionOwnershipGuard::acquire_as(dir.path(), "stamped-key", 4242)
        .expect("first acquire must succeed");
    assert!(
        guard.stamp_path().exists(),
        "acquiring a session key must write an ownership stamp sidecar"
    );
    let contents = std::fs::read_to_string(guard.stamp_path()).expect("read stamp");
    assert!(
        contents.contains("4242"),
        "ownership stamp must record the owning pid, got: {contents:?}"
    );
}

#[test]
fn second_acquire_of_live_owned_key_is_refused_with_clear_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let live_pid = std::process::id();
    let _guard = SessionOwnershipGuard::acquire_as(dir.path(), "shared-key", live_pid)
        .expect("first acquire must succeed");

    let second = SessionOwnershipGuard::acquire_as(dir.path(), "shared-key", live_pid);
    let err = match second {
        Err(e) => e.to_string(),
        Ok(_) => panic!("second acquire of a key owned by a live process must be refused"),
    };
    assert!(
        err.contains("shared-key"),
        "refusal must name the session key, got: {err}"
    );
    assert!(
        err.contains(&live_pid.to_string()),
        "refusal must name the owning pid, got: {err}"
    );
}

#[test]
fn stamp_left_by_dead_process_is_reclaimed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let stamp = ownership_stamp_path(dir.path(), "stale-key");
    std::fs::write(&stamp, dead_pid().to_string()).expect("write stale stamp");

    let claimant_pid = std::process::id();
    let guard = SessionOwnershipGuard::acquire_as(dir.path(), "stale-key", claimant_pid)
        .expect("a stamp left by a dead process must be reclaimable");
    let contents = std::fs::read_to_string(guard.stamp_path()).expect("read stamp");
    assert!(
        contents.contains(&claimant_pid.to_string()),
        "reclaiming must rewrite the stamp with the new owner pid, got: {contents:?}"
    );
}

#[test]
fn dropping_the_guard_releases_ownership() {
    let dir = tempfile::tempdir().expect("tempdir");
    let guard = SessionOwnershipGuard::acquire_as(dir.path(), "released-key", std::process::id())
        .expect("first acquire must succeed");
    let stamp = guard.stamp_path().to_path_buf();
    assert!(stamp.exists(), "acquire must create the ownership stamp");
    drop(guard);
    assert!(
        !stamp.exists(),
        "dropping the guard must remove the ownership stamp"
    );
    SessionOwnershipGuard::acquire_as(dir.path(), "released-key", std::process::id())
        .expect("a released key must be reacquirable");
}
