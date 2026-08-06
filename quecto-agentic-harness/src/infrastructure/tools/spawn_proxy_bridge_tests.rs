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
    bridge.abort();
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
    bridge.abort();
}

#[tokio::test]
async fn bridge_socket_path_is_distinct_from_the_requested_direct_path() {
    let dir = temp_socket_dir();
    let requested = dir.path().join("quecto-agent-abc.sock");
    let bridge = materialize(vec!["cat".to_string()], dir.path(), "abc").unwrap();
    assert_ne!(bridge.socket_path, requested);
    bridge.abort();
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
    bridge.abort();
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
    bridge.abort();
}
