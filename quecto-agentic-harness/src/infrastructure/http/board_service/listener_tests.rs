use super::*;
use crate::infrastructure::http::board_service::test_server::{
    SERVER_UID, raw, request, start, start_tcp, start_with, whole,
};
use std::io::Write;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::UnixStream as StdStream;
use std::sync::atomic::Ordering;
use std::time::Instant;

const KEPT_ALIVE: &[u8] = b"GET /v1/thing HTTP/1.1\r\nHost: board\r\n\r\n";

/// Review round 2: a connection lives at most its lifetime, then closes,
/// though the peer would keep it (the head timeout alone takes 10 s).
#[test]
fn a_connection_closes_at_the_end_of_its_lifetime() {
    let lifetime = Duration::from_millis(300);
    let limits = ConnectionLimits {
        lifetime,
        ..ConnectionLimits::PRODUCTION
    };
    let running = start_with(SERVER_UID, 0, limits);
    let began = Instant::now();
    let answer = whole(&running.socket, KEPT_ALIVE);
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    assert!(
        began.elapsed() < Duration::from_secs(5),
        "{:?}",
        began.elapsed()
    );
}

/// Review round 2: a connection takes at most `MAX_REQUESTS` requests; the
/// last is answered `Connection: close` and the rest are never read.
#[test]
fn a_connection_closes_after_its_most_requests() {
    let running = start(SERVER_UID);
    let answer = whole(
        &running.socket,
        &KEPT_ALIVE.repeat(limits::MAX_REQUESTS + 1),
    );
    assert_eq!(answer.matches("HTTP/1.1 200").count(), limits::MAX_REQUESTS);
    let last = answer.rsplit("HTTP/1.1 200").next().unwrap_or_default();
    assert!(last.contains("connection: close"), "{last}");
    assert_eq!(
        running.handler.calls.load(Ordering::SeqCst),
        limits::MAX_REQUESTS
    );
}

/// Review round 2: a method outside the allowlist is 405 naming what its
/// path takes, and 404 on a path no route has.
#[test]
fn a_refused_method_names_what_its_path_takes() {
    let running = start(SERVER_UID);
    let answer = whole(&running.socket, &request("DELETE", 2, "", b""));
    assert!(answer.starts_with("HTTP/1.1 405"), "{answer}");
    assert!(answer.contains("allow: GET, POST"), "{answer}");
    let nowhere = b"DELETE /nowhere HTTP/1.1\r\nHost: board\r\nConnection: close\r\n\r\n";
    assert_eq!(raw(&running.socket, nowhere).0, 404);
}

/// Review round 1: 64 peers that pipeline requests and never read their
/// replies held every slot for good. Now each loses its connection once
/// it takes no reply byte for the write timeout, and a fresh client is
/// answered soon after.
#[test]
fn peers_that_never_read_free_their_slots_for_a_fresh_client() {
    let write_timeout = Duration::from_millis(500);
    let limits = ConnectionLimits {
        write_timeout,
        ..ConnectionLimits::PRODUCTION
    };
    let running = start_with(SERVER_UID, 64 * 1024, limits);
    let one = b"GET /v1/thing HTTP/1.1\r\nHost: board\r\n\r\n";
    let pipelined: Vec<u8> = one.repeat(200);
    let mut held = Vec::new();
    let mut writers = Vec::new();
    for _ in 0..limits::MAX_CONNECTIONS {
        let peer = StdStream::connect(&running.socket).unwrap();
        let mut writer = peer.try_clone().unwrap();
        let bytes = pipelined.clone();
        writers.push(std::thread::spawn(move || {
            let _ = writer.write_all(&bytes);
        }));
        held.push(peer);
    }
    let began = Instant::now();
    let (status, body) = raw(&running.socket, &request("GET", 2, "", b""));
    assert_eq!(status, 200, "{body}");
    assert!(
        began.elapsed() < Duration::from_secs(5),
        "answered in {:?}",
        began.elapsed()
    );
    for peer in &held {
        let _ = peer.shutdown(std::net::Shutdown::Both);
    }
    for writer in writers {
        writer.join().unwrap();
    }
}

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
    assert!(body.contains(r#""code":"peer""#), "{body}");
    assert_eq!(running.handler.calls.load(Ordering::SeqCst), 0);
}

/// Review round 4: the HTTP layer's own refusals (hyper's) have no body:
/// 431 for a head past its bounds, 400 for one it cannot parse. Clients
/// map them by status (`head_too_large`, `bad_request`).
#[test]
fn the_http_layers_refusals_are_bodyless() {
    let running = start(SERVER_UID);
    for (sent, status) in [
        (request("GET", 33, "", b""), "431"),
        (b"NOT HTTP AT ALL\r\n\r\n".to_vec(), "400"),
        (request("POST", 3, "Content-Length: ten\r\n", b""), "400"),
    ] {
        let answer = whole(&running.socket, &sent);
        assert!(
            answer.starts_with(&format!("HTTP/1.1 {status}")),
            "{answer}"
        );
        let body = answer.split_once("\r\n\r\n").map(|(_, body)| body);
        assert_eq!(body, Some(""), "{answer}");
    }
    assert_eq!(running.handler.calls.load(Ordering::SeqCst), 0);
}

/// Review round 4: hyper merges two equal `Content-Length` headers into
/// one length (and refuses unequal ones, bodyless), so `admit` judges the
/// one length hyper read and never a header's text.
#[test]
fn two_equal_lengths_are_one_length() {
    let running = start(SERVER_UID);
    let twice = "Content-Length: 1\r\nContent-Length: 1\r\n";
    let (status, body) = raw(&running.socket, &request("POST", 4, twice, b"a"));
    assert_eq!(status, 200, "{body}");
    assert!(body.contains(r#""body":1"#), "{body}");
    let unequal = "Content-Length: 1\r\nContent-Length: 2\r\n";
    let answer = whole(&running.socket, &request("POST", 4, unequal, b"ab"));
    assert!(answer.starts_with("HTTP/1.1 400"), "{answer}");
}

/// Review round 3: every error reply the listener makes has a typed code.
#[test]
fn each_listener_refusal_has_its_code() {
    let running = start(SERVER_UID);
    let chunked = request("POST", 3, "Transfer-Encoding: chunked\r\n", b"0\r\n\r\n");
    let over = request("POST", 3, "Content-Length: 1048577\r\n", b"");
    let nowhere = b"DELETE /nowhere HTTP/1.1\r\nHost: board\r\nConnection: close\r\n\r\n";
    for (sent, code) in [
        (request("DELETE", 2, "", b""), "method"),
        (nowhere.to_vec(), "route"),
        (chunked, "length"),
        (over, "too_large"),
    ] {
        let (_, body) = raw(&running.socket, &sent);
        assert!(
            body.contains(&format!(r#""code":"{code}""#)),
            "{code}: {body}"
        );
    }
}

/// Review round 3: past its lifetime a connection has the grace to finish
/// what it answers, and no more: a peer that never reads its reply has it
/// cut short, though the write deadline alone would wait far longer.
#[test]
fn a_connection_past_its_life_is_dropped_after_its_grace() {
    let limits = ConnectionLimits {
        write_timeout: Duration::from_secs(60),
        lifetime: Duration::from_millis(200),
        grace: Duration::from_millis(200),
    };
    let pad = 4 * 1024 * 1024;
    let running = start_with(SERVER_UID, pad, limits);
    let mut peer = StdStream::connect(&running.socket).unwrap();
    peer.write_all(&request("GET", 2, "", b"")).unwrap();
    std::thread::sleep(Duration::from_secs(1));
    peer.set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut answer = Vec::new();
    let _ = std::io::Read::read_to_end(&mut peer, &mut answer);
    assert!(
        answer.len() < pad,
        "the reply was cut short: {} bytes",
        answer.len()
    );
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
    // A test running beside this one may fork a child (git) while the
    // listener is open: until that child execs, it holds the descriptor
    // and the socket still listens. So the stale socket is replaced once
    // nothing holds it, within a bound.
    let rebound = (0..100).find_map(|_| match bind(&stale) {
        Ok(listener) => Some(Ok(listener)),
        Err(BindError::AlreadyServing(_)) => {
            std::thread::sleep(Duration::from_millis(20));
            None
        }
        Err(error) => Some(Err(error)),
    });
    assert!(matches!(rebound, Some(Ok(_))), "{rebound:?}");
}

/// Sends `request` over TCP to `address` and reads the answer to its end.
fn over_tcp(address: std::net::SocketAddr, request: &[u8]) -> String {
    use std::io::Read;
    let mut stream = std::net::TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let _ = stream.write_all(request);
    let mut answer = Vec::new();
    let _ = stream.read_to_end(&mut answer);
    String::from_utf8_lossy(&answer).to_string()
}

/// B5 red: peer credentials are the Unix socket's alone. A TCP peer is
/// served (a Unix peer of the same uid is refused), each request handed on
/// as TCP with its one `Authorization`; a Unix request is local.
#[test]
fn tcp_requests_reach_the_handler_as_tcp_with_their_authorization() {
    let running = start_tcp(SERVER_UID + 1);
    let address = running.tcp.unwrap();
    assert!(address.ip().is_loopback());
    let bearer = b"GET /v1/thing HTTP/1.1\r\nHost: board\r\nAuthorization: Bearer abc\r\n\
                   Connection: close\r\n\r\n";
    let answer = over_tcp(address, bearer);
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    assert!(answer.contains(r#""tcp":true"#), "{answer}");
    assert!(
        answer.contains(r#""authorization":"Bearer abc""#),
        "{answer}"
    );
    let twice = b"GET /v1/thing HTTP/1.1\r\nHost: board\r\nAuthorization: Bearer a\r\n\
                  Authorization: Bearer b\r\nConnection: close\r\n\r\n";
    assert!(over_tcp(address, twice).contains(r#""authorization":"""#));
    let (status, body) = raw(&running.socket, &request("GET", 2, "", b""));
    assert_eq!(status, 403, "{body}");
    assert_eq!(running.handler.calls.load(Ordering::SeqCst), 2);
}

/// TCP peers that hold every TCP slot never lock out the Unix socket: each
/// listener has its own slots.
#[test]
fn tcp_peers_holding_every_slot_leave_the_socket_served() {
    let running = start_tcp(SERVER_UID);
    let address = running.tcp.unwrap();
    let held: Vec<_> = (0..limits::MAX_CONNECTIONS + 4)
        .map(|_| std::net::TcpStream::connect(address).unwrap())
        .collect();
    std::thread::sleep(Duration::from_millis(200));
    let began = Instant::now();
    let (status, body) = raw(&running.socket, &request("GET", 2, "", b""));
    assert_eq!(status, 200, "{body}");
    assert!(
        began.elapsed() < Duration::from_secs(5),
        "{:?}",
        began.elapsed()
    );
    drop(held);
}

#[test]
fn unix_requests_reach_the_handler_as_local() {
    let running = start(SERVER_UID);
    let (status, body) = raw(&running.socket, &request("GET", 2, "", b""));
    assert_eq!(status, 200, "{body}");
    assert!(body.contains(r#""tcp":false"#), "{body}");
    assert!(body.contains(r#""authorization":null"#), "{body}");
}
