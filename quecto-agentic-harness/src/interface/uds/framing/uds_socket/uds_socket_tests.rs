use super::*;
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

#[test]
fn reap_stale_sockets_removes_dead_matching_socket_files_only() {
    let dir = tempfile::tempdir().unwrap();
    let dead = dir.path().join("quecto-agent-dead.sock");
    let other = dir.path().join("other.sock");
    std::fs::write(&dead, b"stale").unwrap();
    std::fs::write(&other, b"keep").unwrap();

    reap_stale_sockets(dir.path(), Duration::from_secs(60));

    assert!(!dead.exists());
    assert!(other.exists());
}

#[tokio::test]
async fn reap_stale_sockets_keeps_live_socket_and_bind_sets_owner_only_mode() {
    let dir = tempfile::tempdir().unwrap();
    let live = dir.path().join("quecto-agent-live.sock");
    let listener = bind_secure_socket(&live).unwrap();

    reap_stale_sockets(dir.path(), Duration::from_secs(0));

    assert!(live.exists());
    assert_eq!(
        std::fs::metadata(&live).unwrap().permissions().mode() & 0o777,
        0o600
    );
    drop(listener);
}

#[test]
fn socket_guard_removes_socket_file_on_drop() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("guard.sock");
    std::fs::write(&path, b"socket placeholder").unwrap();

    drop(SocketGuard(path.clone()));

    assert!(!path.exists());
}
