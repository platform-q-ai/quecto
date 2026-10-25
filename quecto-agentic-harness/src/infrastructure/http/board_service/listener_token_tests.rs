//! #2727: each token holds at most its share of TCP connections, from its
//! first authenticated request until the connection closes.
use super::*;
use std::io::Read;
use std::net::{SocketAddr, TcpStream};

/// A request authenticated as `token` that closes its connection.
fn closing_as(token: &str) -> Vec<u8> {
    format!(
        "GET /v1/thing HTTP/1.1\r\nHost: board\r\nAuthorization: Bearer {token}\r\n\
         Connection: close\r\n\r\n"
    )
    .into_bytes()
}

/// Sends `request` on `stream` and reads one whole reply (head, then the
/// body its `content-length` declares), leaving the connection open.
fn reply_on(stream: &mut TcpStream, request: &[u8]) -> String {
    stream.write_all(request).unwrap();
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).unwrap();
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head).to_string();
    let length = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .and_then(|length| length.trim().parse::<usize>().ok())
        .unwrap_or_else(|| panic!("no content-length in {head}"));
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body).unwrap();
    format!("{head}{}", String::from_utf8_lossy(&body))
}

/// A loopback connection authenticated as `token` and kept alive, its
/// first reply read whole.
fn held_as(token: &str, address: SocketAddr) -> TcpStream {
    let mut stream = from_source("127.0.0.1", address);
    let answer = reply_on(&mut stream, &kept_as(token));
    assert!(answer.starts_with("HTTP/1.1 200"), "{token}: {answer}");
    stream
}

/// Whether a fresh connection authenticated as `token` is served, asked
/// again within a bound (the server sees a close as it reads the peer's
/// end, a moment after the peer goes).
fn served_soon(token: &str, address: SocketAddr) -> bool {
    (0..100).any(|_| {
        let answer = exchange(from_source("127.0.0.1", address), &closing_as(token));
        let served = answer.starts_with("HTTP/1.1 200");
        if !served {
            std::thread::sleep(Duration::from_millis(20));
        }
        served
    })
}

/// A refusal that says `busy` and closes its connection.
fn assert_busy_and_closed(answer: &str) {
    assert!(answer.starts_with("HTTP/1.1 503"), "{answer}");
    assert!(answer.contains(r#""code":"busy""#), "{answer}");
    assert!(answer.contains("connection: close"), "{answer}");
}

/// A listener on TCP whose tokens each hold one connection.
fn one_each() -> crate::infrastructure::http::board_service::test_server::Running {
    let limits = ConnectionLimits {
        per_token: 1,
        ..ConnectionLimits::PRODUCTION
    };
    start_tcp_with(SERVER_UID, limits)
}

/// One token holds at most `MAX_CONNECTIONS_PER_TOKEN` TCP connections.
/// With all of them live a 9th is answered 503 `busy` and closed, before
/// its handler runs; another token, and the Unix socket, are served.
#[test]
fn a_token_past_its_connections_is_busy_and_others_are_served() {
    let running = start_tcp(SERVER_UID);
    let address = running.tcp.unwrap();
    let held: Vec<_> = (0..limits::MAX_CONNECTIONS_PER_TOKEN)
        .map(|_| held_as("laptop", address))
        .collect();
    let calls = running.handler.calls.load(Ordering::SeqCst);
    assert_eq!(calls, limits::MAX_CONNECTIONS_PER_TOKEN);
    let began = Instant::now();
    let refused = exchange(from_source("127.0.0.1", address), &kept_as("laptop"));
    assert_busy_and_closed(&refused);
    assert!(
        began.elapsed() < Duration::from_secs(5),
        "closed at once: {:?}",
        began.elapsed()
    );
    assert_eq!(running.handler.calls.load(Ordering::SeqCst), calls);
    let phone = exchange(from_source("127.0.0.1", address), &closing_as("phone"));
    assert!(phone.starts_with("HTTP/1.1 200"), "{phone}");
    let (status, body) = raw(&running.socket, &request("GET", 2, "", b""));
    assert_eq!(status, 200, "{body}");
    drop(held);
}

/// A token's connection gives its place back as it closes, so the token
/// is served on a fresh connection again.
#[test]
fn a_closed_connection_gives_its_tokens_place_back() {
    let running = start_tcp(SERVER_UID);
    let address = running.tcp.unwrap();
    let mut held: Vec<_> = (0..limits::MAX_CONNECTIONS_PER_TOKEN)
        .map(|_| held_as("laptop", address))
        .collect();
    let refused = exchange(from_source("127.0.0.1", address), &kept_as("laptop"));
    assert_busy_and_closed(&refused);
    drop(held.pop());
    assert!(
        served_soon("laptop", address),
        "a place given back is taken again"
    );
    drop(held);
}

/// Review round 1 (M1a): a connection holding token A's one place that
/// switches to token B is served as B, and gives A's place back: A is
/// served on a fresh connection while the switched one stays open.
#[test]
fn a_connection_that_switches_token_gives_the_old_place_back() {
    let running = one_each();
    let address = running.tcp.unwrap();
    let mut switching = held_as("alpha", address);
    let refused = exchange(from_source("127.0.0.1", address), &kept_as("alpha"));
    assert_busy_and_closed(&refused);
    let answer = reply_on(&mut switching, &kept_as("beta"));
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    assert!(answer.contains(r#""principal":"Bearer beta""#), "{answer}");
    assert!(
        served_soon("alpha", address),
        "alpha's place was given back"
    );
    let refused = exchange(from_source("127.0.0.1", address), &kept_as("beta"));
    assert_busy_and_closed(&refused);
    drop(switching);
}

/// Review round 1 (M1b): a connection on token A that switches to token
/// B, whose one place is held, is answered 503 `busy` and closed, and
/// A's place is given back with it.
#[test]
fn a_switch_to_a_full_token_is_busy_and_gives_the_old_place_back() {
    let running = one_each();
    let address = running.tcp.unwrap();
    let beta = held_as("beta", address);
    let mut switching = held_as("alpha", address);
    let calls = running.handler.calls.load(Ordering::SeqCst);
    switching.write_all(&kept_as("beta")).unwrap();
    let mut answer = Vec::new();
    let _ = switching.read_to_end(&mut answer);
    let answer = String::from_utf8_lossy(&answer).to_string();
    assert_busy_and_closed(&answer);
    assert_eq!(running.handler.calls.load(Ordering::SeqCst), calls);
    assert!(
        served_soon("alpha", address),
        "alpha's place was given back"
    );
    drop(beta);
}

/// Review round 1 (L2): a capped request's body is read before it is
/// refused, so its 503 `busy` arrives whole, and only once the body is
/// sent: never a reset to a client whose body comes a moment late.
#[test]
fn a_capped_request_is_refused_after_its_body_is_read() {
    let running = one_each();
    let address = running.tcp.unwrap();
    let held = held_as("alpha", address);
    let mut late = from_source("127.0.0.1", address);
    let head = "POST /v1/thing HTTP/1.1\r\nHost: board\r\nAuthorization: Bearer alpha\r\n\
                Content-Length: 5\r\n\r\n";
    late.write_all(head.as_bytes()).unwrap();
    late.set_read_timeout(Some(Duration::from_millis(300)))
        .unwrap();
    let mut early = [0u8; 1];
    let before = late.read(&mut early);
    assert!(
        before.is_err(),
        "nothing is answered before the body: {before:?}"
    );
    late.set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    late.write_all(b"hello").unwrap();
    let mut answer = Vec::new();
    let _ = late.read_to_end(&mut answer);
    let answer = String::from_utf8_lossy(&answer).to_string();
    assert_busy_and_closed(&answer);
    let body = answer.split_once("\r\n\r\n").map_or("", |(_, body)| body);
    let body: serde_json::Value = serde_json::from_str(body)
        .unwrap_or_else(|error| panic!("the body arrives whole ({error}): {answer}"));
    assert_eq!(body["code"], "busy", "{answer}");
    let why = "too many connections for this token: close one, or try again shortly";
    assert_eq!(body["error"], why, "{answer}");
    drop(held);
}

/// Review round 2 (H1): connections of a full token whose bodies never
/// come stay counted among their source's unauthenticated connections
/// (8, the oldest evicted, each closed past its first-request time), so
/// however many there are they never fill the slots: another token is
/// served at once.
#[test]
fn slow_bodies_on_a_full_token_never_fill_the_slots() {
    let running = one_each();
    let address = running.tcp.unwrap();
    let held = held_as("alpha", address);
    let head = "POST /v1/thing HTTP/1.1\r\nHost: board\r\nAuthorization: Bearer alpha\r\n\
                Content-Length: 5\r\n\r\n";
    let slow: Vec<_> = (0..limits::MAX_CONNECTIONS)
        .map(|_| {
            let mut stream = from_source("127.0.0.1", address);
            stream.write_all(head.as_bytes()).unwrap();
            // Its head reaches the gate before the next connects, so
            // newcomers never evict it before it has authenticated.
            std::thread::sleep(Duration::from_millis(15));
            stream
        })
        .collect();
    std::thread::sleep(Duration::from_millis(300));
    let began = Instant::now();
    let mut beta = from_source("127.0.0.1", address);
    beta.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let answer = exchange(beta, &closing_as("beta"));
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer:?}");
    let took = began.elapsed();
    assert!(took < Duration::from_secs(5), "served in {took:?}");
    drop((held, slow));
}

/// Review round 2 (M1, L1): a request refused by its head still takes
/// its connection's place, kept for the connection's life; on a full
/// token the same refusal is answered as itself (never a retryable
/// `busy`) and closes its connection, taking no place.
#[test]
fn a_refused_head_holds_its_place_and_on_a_full_token_is_closed_as_itself() {
    let running = one_each();
    let address = running.tcp.unwrap();
    let nowhere = b"DELETE /nowhere HTTP/1.1\r\nHost: board\r\nAuthorization: Bearer alpha\r\n\r\n";
    let mut first = from_source("127.0.0.1", address);
    let answer = reply_on(&mut first, nowhere);
    assert!(answer.starts_with("HTTP/1.1 404"), "{answer}");
    assert!(!answer.contains("connection: close"), "{answer}");
    let refused = exchange(from_source("127.0.0.1", address), &kept_as("alpha"));
    assert_busy_and_closed(&refused);
    let over = "POST /v1/thing HTTP/1.1\r\nHost: board\r\nAuthorization: Bearer alpha\r\n\
                Content-Length: 1048577\r\n\r\n";
    for (sent, status, code) in [
        (&nowhere[..], "404", "route"),
        (over.as_bytes(), "413", "too_large"),
    ] {
        let answer = exchange(from_source("127.0.0.1", address), sent);
        assert!(
            answer.starts_with(&format!("HTTP/1.1 {status}")),
            "{answer}"
        );
        assert!(answer.contains(&format!(r#""code":"{code}""#)), "{answer}");
        assert!(answer.contains("connection: close"), "{answer}");
    }
    let answer = reply_on(&mut first, &kept_as("alpha"));
    assert!(answer.starts_with("HTTP/1.1 200"), "still served: {answer}");
    let calls = running.handler.calls.load(Ordering::SeqCst);
    assert_eq!(calls, 1, "no refusal reached the handler");
}
