use super::*;
use crate::infrastructure::http::board_service::test_server::{SERVER_UID, start, start_tcp};
use std::sync::atomic::Ordering;

const TOKEN: &str = "c2VjcmV0LXRva2VuLWZvci10ZXN0cw";

#[test]
fn a_request_over_the_socket_answers_the_status_and_body() {
    let running = start(SERVER_UID);
    let client = BoardServiceClient::unix(&running.socket);
    let reply = client
        .send_read(Method::Post, "/v1/thing", b"{\"a\":1}")
        .unwrap();
    assert_eq!(reply.status, 200);
    let body: serde_json::Value = serde_json::from_slice(&reply.body).unwrap();
    assert_eq!(
        (&body["path"], &body["body"]),
        (&"/v1/thing".into(), &7.into())
    );
    // Each request on its own connection, the server answers them all.
    let again = client.send_read(Method::Get, "/v1/thing", b"").unwrap();
    assert_eq!(again.status, 200);
    assert_eq!(running.handler.calls.load(Ordering::SeqCst), 2);
}

/// Over TCP the token goes as `Authorization: Bearer`; a client's `Debug`
/// never shows it.
#[test]
fn over_tcp_the_token_is_sent_and_never_shown() {
    let running = start_tcp(SERVER_UID);
    let token = BearerToken::parse(TOKEN).unwrap();
    let client = BoardServiceClient::new(Endpoint::Tcp(running.tcp.unwrap()), Some(token));
    let reply = client.send_read(Method::Get, "/v1/thing", b"").unwrap();
    let body: serde_json::Value = serde_json::from_slice(&reply.body).unwrap();
    assert_eq!(body["principal"], format!("Bearer {TOKEN}"), "{body}");
    assert!(!format!("{client:?}").contains(TOKEN));
    for refused in ["short", "has space in it here", "new\r\nline-token-000000"] {
        assert_eq!(BearerToken::parse(refused), None, "{refused:?}");
    }
}

#[test]
fn a_refusal_is_a_reply_not_an_error() {
    let running = start(SERVER_UID + 1);
    let reply = BoardServiceClient::unix(&running.socket)
        .send_read(Method::Get, "/v1/status", b"")
        .unwrap();
    assert_eq!(reply.status, 403);
}

#[test]
fn no_server_is_a_connect_error_a_retry_may_mend() {
    let dir = tempfile::tempdir().unwrap();
    let error = BoardServiceClient::unix(&dir.path().join("none.sock"))
        .send_read(Method::Get, "/v1/status", b"")
        .unwrap_err();
    assert!(matches!(error, ClientError::Connect(_)), "{error}");
    assert!(error.transport());
    assert!(!ClientError::Malformed("x".into()).transport());
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
        let error = client.send_read(Method::Get, path, b"").unwrap_err();
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

/// A request sent within a read's reply time.
trait SendRead {
    fn send_read(
        &self,
        method: crate::infrastructure::http::board_service::limits::Method,
        path: &str,
        body: &[u8],
    ) -> Result<
        crate::infrastructure::http::board_service::client::ClientReply,
        crate::infrastructure::http::board_service::client::ClientError,
    >;
}

impl SendRead for crate::infrastructure::http::board_service::client::BoardServiceClient {
    fn send_read(
        &self,
        method: crate::infrastructure::http::board_service::limits::Method,
        path: &str,
        body: &[u8],
    ) -> Result<
        crate::infrastructure::http::board_service::client::ClientReply,
        crate::infrastructure::http::board_service::client::ClientError,
    > {
        let reply = crate::infrastructure::http::board_service::client::READ_REPLY;
        self.send(method, path, body, reply)
    }
}
