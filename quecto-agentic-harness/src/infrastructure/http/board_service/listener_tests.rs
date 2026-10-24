use super::*;
use crate::infrastructure::http::board_service::test_server::{SERVER_UID, raw, request, start};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::sync::atomic::Ordering;

#[test]
fn the_same_uid_is_answered_by_the_handler() {
    let running = start(SERVER_UID);
    let (status, body) = raw(&running.socket, &request("GET", 2, "", b""));
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("\"path\":\"/v1/thing\""), "{body}");
    assert_eq!(running.handler.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn a_peer_of_another_uid_is_refused_before_the_handler() {
    let running = start(SERVER_UID + 1);
    let (status, body) = raw(&running.socket, &request("GET", 2, "", b""));
    assert_eq!(status, 403, "{body}");
    assert_eq!(running.handler.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn thirty_two_headers_are_read_and_thirty_three_are_431() {
    let running = start(SERVER_UID);
    let (status, _) = raw(&running.socket, &request("GET", 32, "", b""));
    assert_eq!(status, 200);
    let (status, _) = raw(&running.socket, &request("GET", 33, "", b""));
    assert_eq!(status, 431);
    assert_eq!(running.handler.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn a_body_of_one_mebibyte_is_read_and_one_byte_more_is_413() {
    let running = start(SERVER_UID);
    let most = 1024 * 1024;
    let body = vec![b'a'; most];
    let (status, answer) = raw(
        &running.socket,
        &request("POST", 3, &format!("Content-Length: {most}\r\n"), &body),
    );
    assert_eq!(status, 200, "{answer}");
    assert!(answer.contains(&format!("\"body\":{most}")), "{answer}");
    let over = most + 1;
    let (status, _) = raw(
        &running.socket,
        &request("POST", 3, &format!("Content-Length: {over}\r\n"), b""),
    );
    assert_eq!(status, 413);
    assert_eq!(running.handler.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn a_method_outside_the_allowlist_is_405() {
    let running = start(SERVER_UID);
    for method in ["DELETE", "PUT", "BREW", "OPTIONS"] {
        let (status, _) = raw(&running.socket, &request(method, 2, "", b""));
        assert_eq!(status, 405, "{method}");
    }
    assert_eq!(running.handler.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn a_chunked_body_is_refused_as_length_required() {
    let running = start(SERVER_UID);
    let (status, _) = raw(
        &running.socket,
        &request(
            "POST",
            3,
            "Transfer-Encoding: chunked\r\n",
            b"1\r\na\r\n0\r\n\r\n",
        ),
    );
    assert_eq!(status, 411);
    assert_eq!(running.handler.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn the_socket_is_0600_in_a_0700_directory() {
    let running = start(SERVER_UID);
    let socket = std::fs::symlink_metadata(&running.socket).unwrap();
    assert!(socket.file_type().is_socket());
    assert_eq!(socket.permissions().mode() & 0o777, 0o600);
    let dir = std::fs::metadata(running.socket.parent().unwrap()).unwrap();
    assert_eq!(dir.permissions().mode() & 0o777, 0o700);
}

#[test]
fn a_path_that_is_not_a_socket_is_never_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("board");
    std::fs::create_dir(&board).unwrap();
    std::fs::set_permissions(&board, std::fs::Permissions::from_mode(0o700)).unwrap();
    let file = board.join("board.sock");
    std::fs::write(&file, "keep me").unwrap();
    let error = bind(&file).unwrap_err();
    assert!(error.to_string().contains("not a socket"), "{error}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "keep me");
}

#[test]
fn a_directory_open_to_others_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("board");
    std::fs::create_dir(&board).unwrap();
    std::fs::set_permissions(&board, std::fs::Permissions::from_mode(0o755)).unwrap();
    let error = bind(&board.join("board.sock")).unwrap_err();
    assert!(error.to_string().contains("0700"), "{error}");
}

#[test]
fn a_relative_socket_path_is_refused() {
    let error = bind(std::path::Path::new("board/board.sock")).unwrap_err();
    assert!(error.to_string().contains("absolute"), "{error}");
}

#[test]
fn a_live_socket_is_refused_and_a_stale_one_replaced() {
    let running = start(SERVER_UID);
    let error = bind(&running.socket).unwrap_err();
    assert!(error.to_string().contains("already serving"), "{error}");
    // The live server still answers.
    let (status, _) = raw(&running.socket, &request("GET", 2, "", b""));
    assert_eq!(status, 200);
    let dir = tempfile::tempdir().unwrap();
    let stale = dir.path().join("board").join("board.sock");
    drop(bind(&stale).unwrap());
    // The listener is closed, its file left behind: a stale socket.
    assert!(std::fs::symlink_metadata(&stale).is_ok());
    assert!(bind(&stale).is_ok());
}
