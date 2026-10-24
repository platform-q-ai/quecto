use super::*;
use crate::infrastructure::http::board_service::test_server::{SERVER_UID, start};

#[test]
fn an_answer_is_its_status_and_body() {
    let answer = parsed(b"HTTP/1.1 201 Created\r\ncontent-length: 2\r\n\r\n{}").unwrap();
    assert_eq!(answer.status, 201);
    assert_eq!(answer.body, b"{}");
    for refused in [
        &b""[..],
        b"HTTP/1.1 201 Created",
        b"HTTP/1.0 200 OK\r\n\r\n",
        b"HTTP/1.1 2x0 OK\r\n\r\n",
    ] {
        assert_eq!(parsed(refused), None, "{refused:?}");
    }
}

#[test]
fn a_request_reaches_the_server_with_its_body() {
    let running = start(SERVER_UID);
    let answer = local_request(&running.socket, Method::Post, "/v1/thing", b"{\"a\":1}").unwrap();
    assert_eq!(answer.status, 200);
    let body: serde_json::Value = serde_json::from_slice(&answer.body).unwrap();
    assert_eq!(body["path"], "/v1/thing");
    assert_eq!(body["body"], 7);
}

#[test]
fn no_server_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        local_request(
            &dir.path().join("none.sock"),
            Method::Get,
            "/v1/status",
            b""
        )
        .is_err()
    );
}
