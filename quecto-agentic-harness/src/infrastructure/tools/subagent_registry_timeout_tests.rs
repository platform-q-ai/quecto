use super::*;
use std::time::Duration;

#[tokio::test]
async fn stalled_request_write_is_inside_operation_deadline() {
    let tmp = tempfile::tempdir().unwrap();
    let socket = tmp.path().join("write.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let server = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        std::future::pending::<()>().await;
    });
    let command =
        serde_json::json!({"type":"get_state","padding":"x".repeat(2_000_000)}).to_string();
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        send_subagent_uds_command_with_timeout(&socket, &command, Duration::from_millis(50)),
    )
    .await;
    server.abort();
    let error = result
        .expect("blocked writes must share the inspection deadline")
        .unwrap_err();
    assert!(error.to_string().contains("timed out"), "{error}");
}

#[tokio::test]
async fn unavailable_connection_has_clear_bounded_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let error = send_subagent_uds_command_with_timeout(
        &tmp.path().join("missing.sock"),
        r#"{"type":"get_state"}"#,
        Duration::from_millis(50),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("connect to subagent"), "{error}");
}

#[tokio::test]
async fn invalid_responses_do_not_extend_read_deadline() {
    use tokio::io::AsyncWriteExt;
    let tmp = tempfile::tempdir().unwrap();
    let socket = tmp.path().join("invalid.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        loop {
            if stream.write_all(b"{\"type\":\"response\",\"command\":\"get_state\",\"data\":{\"unknown\":true}}\n").await.is_ok() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            } else {
                break;
            }
        }
    });
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        send_subagent_uds_command_with_timeout(
            &socket,
            r#"{"type":"get_state"}"#,
            Duration::from_millis(50),
        ),
    )
    .await;
    server.abort();
    assert!(
        result
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("timed out")
    );
}

#[tokio::test]
async fn long_sender_keeps_waiting_beyond_inspection_deadline() {
    let tmp = tempfile::tempdir().unwrap();
    let socket = tmp.path().join("long.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let server = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        std::future::pending::<()>().await;
    });
    let result = tokio::time::timeout(
        INSPECTOR_RESPONSE_TIMEOUT + Duration::from_millis(100),
        send_subagent_uds_command(&socket, r#"{"type":"get_messages"}"#),
    )
    .await;
    server.abort();
    assert!(
        result.is_err(),
        "long operation must retain its independent 300s response budget"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn saturated_accept_queue_has_bounded_connect_outcome() {
    use std::os::fd::AsRawFd;
    let tmp = tempfile::tempdir().unwrap();
    let socket = tmp.path().join("connect.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    // Linux permits one queued connection at backlog zero. Keep that slot
    // occupied without accepting so the inspection cannot connect normally.
    // SAFETY: listener owns a valid socket fd throughout this call.
    assert_eq!(unsafe { libc::listen(listener.as_raw_fd(), 0) }, 0);
    let _queued = std::os::unix::net::UnixStream::connect(&socket).unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        send_subagent_uds_command_with_timeout(
            &socket,
            r#"{"type":"get_state"}"#,
            Duration::from_millis(50),
        ),
    )
    .await
    .expect("saturated connection queue must not hang inspection");
    let error = result.unwrap_err().to_string();
    assert!(
        error.contains("timed out") || error.contains("connect to subagent"),
        "{error}"
    );
}

#[tokio::test]
async fn pending_connect_is_cancelled_by_the_shared_operation_deadline() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let polled = AtomicBool::new(false);
    let cancelled = AtomicBool::new(false);
    struct CancelProbe<'a>(&'a AtomicBool);
    impl Drop for CancelProbe<'_> {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let connect = async {
        let _probe = CancelProbe(&cancelled);
        polled.store(true, Ordering::SeqCst);
        std::future::pending::<std::io::Result<tokio::net::UnixStream>>().await
    };
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        with_operation_deadline(
            Duration::from_millis(50),
            send_subagent_uds_command_connected(
                std::path::Path::new("pending-connect.sock"),
                r#"{"type":"get_state"}"#,
                Duration::from_millis(50),
                connect,
            ),
        ),
    )
    .await
    .expect("pending connection must be bounded by the real operation wrapper");
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("connect/write/read deadline")
    );
    assert!(polled.load(Ordering::SeqCst), "connect phase was exercised");
    assert!(
        cancelled.load(Ordering::SeqCst),
        "timed-out connection future must be dropped"
    );
}
