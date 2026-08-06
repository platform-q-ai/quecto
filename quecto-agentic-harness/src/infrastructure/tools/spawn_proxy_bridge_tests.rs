//! Behavioral tests for the parent-side proxy bridge (#1369 slice 3), using
//! real UNIX listeners and real proxy processes.

use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn temp_socket_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("temp socket dir")
}

#[tokio::test]
async fn bridge_pumps_both_directions_through_the_proxy_process() {
    let dir = temp_socket_dir();
    // `cat` is a real stdio process: bytes written into the bridge come back.
    let bridge = materialize(vec!["cat".to_string()], dir.path(), "echo-agent").unwrap();
    let mut conn = tokio::net::UnixStream::connect(&bridge.socket_path)
        .await
        .unwrap();
    conn.write_all(b"ping-through-proxy").await.unwrap();
    let mut buf = [0u8; 18];
    conn.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"ping-through-proxy");
    bridge.teardown();
}

#[tokio::test]
async fn proxy_death_is_pushed_to_the_parent_as_eof() {
    let dir = temp_socket_dir();
    // A proxy that exits immediately models a dead child/environment: the
    // parent-side connection must observe EOF, not hang.
    let bridge = materialize(vec!["true".to_string()], dir.path(), "dead-agent").unwrap();
    let mut conn = tokio::net::UnixStream::connect(&bridge.socket_path)
        .await
        .unwrap();
    let mut buf = [0u8; 1];
    let n = tokio::time::timeout(std::time::Duration::from_secs(5), conn.read(&mut buf))
        .await
        .expect("EOF must be pushed promptly")
        .unwrap();
    assert_eq!(n, 0, "dead proxy must read as EOF");
    bridge.teardown();
}

#[tokio::test]
async fn bridge_socket_path_is_distinct_from_the_requested_direct_path() {
    let dir = temp_socket_dir();
    let requested = dir.path().join("quecto-agent-abc.sock");
    let bridge = materialize(vec!["cat".to_string()], dir.path(), "abc").unwrap();
    assert_ne!(bridge.socket_path, requested);
    bridge.teardown();
}

#[tokio::test]
async fn materialize_fails_cleanly_on_unbindable_socket_dir() {
    let err = materialize(
        vec!["cat".to_string()],
        std::path::Path::new("/nonexistent-quecto-bridge-dir"),
        "x",
    )
    .unwrap_err();
    assert!(!err.to_string().is_empty());
}

#[tokio::test]
async fn wait_for_proxy_ready_accepts_a_held_open_connection() {
    let dir = temp_socket_dir();
    // `sleep` holds stdio open without writing: the probe sees no EOF within
    // its window and reports ready.
    let bridge = materialize(
        vec!["sleep".to_string(), "30".to_string()],
        dir.path(),
        "ready-agent",
    )
    .unwrap();
    wait_for_proxy_ready(&bridge.socket_path).await.unwrap();
    bridge.teardown();
}

#[tokio::test]
async fn wait_for_proxy_ready_times_out_without_a_bridge() {
    let dir = temp_socket_dir();
    let missing = dir.path().join("missing-bridge.sock");
    let err = wait_for_proxy_ready(&missing).await.unwrap_err();
    assert!(err.to_string().contains("did not become ready"), "{err}");
}

#[tokio::test]
async fn bridge_one_survives_an_unspawnable_proxy_argv() {
    let dir = temp_socket_dir();
    let bridge = materialize(
        vec!["/nonexistent-proxy-binary".to_string()],
        dir.path(),
        "bad-argv",
    )
    .unwrap();
    let mut conn = tokio::net::UnixStream::connect(&bridge.socket_path)
        .await
        .unwrap();
    let mut buf = [0u8; 1];
    // Connection closes (EOF or reset) rather than hanging or panicking.
    let read = tokio::time::timeout(std::time::Duration::from_secs(5), conn.read(&mut buf)).await;
    assert!(matches!(read, Ok(Ok(0)) | Ok(Err(_))));
    bridge.teardown();
}

/// #1391 review: a bridged connection that the parent drops must tear down
/// its proxy process even when the child side never writes — otherwise every
/// dropped probe/await connection leaks a live proxy for the child's
/// lifetime.
#[tokio::test]
async fn dropped_connection_tears_down_the_proxy_process() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("proxy.pid");
    let proxy = dir.path().join("proxy.sh");
    std::fs::write(
        &proxy,
        format!(
            "#!/usr/bin/env bash\necho $$ > '{}'\nexec sleep 30\n",
            pid_file.display()
        ),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        let mut p = std::fs::metadata(&proxy).unwrap().permissions();
        p.set_mode(0o700);
        std::fs::set_permissions(&proxy, p).unwrap();
    }

    let bridge = materialize(
        vec![proxy.to_string_lossy().to_string()],
        dir.path(),
        "leak-test",
    )
    .unwrap();
    let conn = tokio::net::UnixStream::connect(&bridge.socket_path)
        .await
        .unwrap();
    // Wait for the proxy to start, then drop the parent connection.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    while !pid_file.exists() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let pid: i32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    drop(conn);

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let alive = std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .unwrap()
            .success();
        if !alive {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "proxy process {pid} still alive after parent connection dropped"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    bridge.teardown();
}

#[tokio::test]
async fn into_parts_hands_over_socket_and_handle() {
    let dir = tempfile::tempdir().unwrap();
    let bridge = materialize(vec!["true".to_string()], dir.path(), "parts-test").unwrap();
    let expected = bridge.socket_path.clone();
    let (socket, handle) = bridge.into_parts();
    assert_eq!(socket, expected);
    handle.abort();
    let _ = std::fs::remove_file(&socket);
}
