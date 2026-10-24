use super::*;
use crate::infrastructure::http::board_service::test_server::{SERVER_UID, start};
use std::sync::atomic::Ordering;

#[test]
fn a_get_over_the_socket_answers_the_status_and_body() {
    let running = start(SERVER_UID);
    let client = BoardServiceClient::unix(&running.socket);
    let reply = client.get("/v1/status").unwrap();
    assert_eq!(reply.status, 200);
    let body: serde_json::Value = serde_json::from_slice(&reply.body).unwrap();
    assert_eq!(body["path"], "/v1/status");
    // Each request on its own connection, the server answers them all.
    let again = client.get("/v1/time").unwrap();
    assert_eq!(again.status, 200);
    assert_eq!(running.handler.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn a_refusal_is_a_reply_not_an_error() {
    let running = start(SERVER_UID + 1);
    let reply = BoardServiceClient::unix(&running.socket)
        .get("/v1/status")
        .unwrap();
    assert_eq!(reply.status, 403);
}

#[test]
fn no_server_is_a_connect_error() {
    let dir = tempfile::tempdir().unwrap();
    let error = BoardServiceClient::unix(&dir.path().join("none.sock"))
        .get("/v1/status")
        .unwrap_err();
    assert!(matches!(error, ClientError::Connect(_)), "{error}");
}

#[test]
fn a_path_outside_the_allowlist_is_never_sent() {
    let running = start(SERVER_UID);
    let client = BoardServiceClient::unix(&running.socket);
    for path in [
        "v1/status",
        "/v1/status HTTP/1.1\r\nX: y",
        "/v1/st atus",
        "",
    ] {
        let error = client.get(path).unwrap_err();
        assert!(
            matches!(error, ClientError::Request(_)),
            "{path:?}: {error}"
        );
    }
    assert_eq!(running.handler.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn a_reply_is_parsed_by_its_status_line_and_length() {
    let reply = parse_reply(b"HTTP/1.1 404 Not Found\r\ncontent-length: 2\r\n\r\n{}").unwrap();
    assert_eq!(reply.status, 404);
    assert_eq!(reply.body, b"{}");
    for malformed in [
        &b"HTTP/1.1 200 OK\r\n\r\n{}"[..],
        b"HTTP/1.1 2x0 OK\r\ncontent-length: 2\r\n\r\n{}",
        b"SMTP 200 OK\r\ncontent-length: 2\r\n\r\n{}",
        b"HTTP/1.1 200 OK\r\ncontent-length: 5\r\n\r\n{}",
    ] {
        assert!(
            parse_reply(malformed).is_err(),
            "{}",
            String::from_utf8_lossy(malformed)
        );
    }
}
