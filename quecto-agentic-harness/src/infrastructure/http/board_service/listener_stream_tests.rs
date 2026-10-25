//! Streamed replies (epic #2705 B10): a healthy stream lives on its
//! keep-alives until its connection's lifetime, a stalled reader is let
//! go, and streams hold places of their own beside the connection caps.
use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::unix::net::UnixStream;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use super::ConnectionLimits;
use crate::infrastructure::http::board_service::limits::{MAX_STREAMS, MAX_STREAMS_PER_TOKEN};
use crate::infrastructure::http::board_service::test_server::{
    SERVER_UID, raw, start_tcp_with, start_with, whole,
};

const STREAM: &[u8] = b"GET /v1/stream HTTP/1.1\r\nHost: board\r\n\r\n";

/// Reads what `stream` sends until `until` passes or it closes: the
/// text, and whether it closed.
fn read_for(stream: &mut impl Read, until: Duration) -> (String, bool) {
    let began = Instant::now();
    let mut seen = Vec::new();
    let mut buf = [0u8; 4096];
    while began.elapsed() < until {
        match stream.read(&mut buf) {
            Ok(0) => return (String::from_utf8_lossy(&seen).into(), true),
            Ok(read) => seen.extend_from_slice(&buf[..read]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {}
            Err(_) => return (String::from_utf8_lossy(&seen).into(), true),
        }
    }
    (String::from_utf8_lossy(&seen).into(), false)
}

/// Reads what `stream` sends until its reply's head (and the start of its
/// body) is in, it closes, or 2 s pass: the text, and whether it closed.
fn read_head(stream: &mut impl Read) -> (String, bool) {
    let began = Instant::now();
    let mut seen = Vec::new();
    let mut buf = [0u8; 4096];
    while began.elapsed() < Duration::from_secs(2) && !seen.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut buf) {
            Ok(0) => return (String::from_utf8_lossy(&seen).into(), true),
            Ok(read) => seen.extend_from_slice(&buf[..read]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {}
            Err(_) => return (String::from_utf8_lossy(&seen).into(), true),
        }
    }
    (String::from_utf8_lossy(&seen).into(), false)
}

fn unix_stream(socket: &std::path::Path, request: &[u8]) -> UnixStream {
    let mut stream = UnixStream::connect(socket).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    stream.write_all(request).unwrap();
    stream
}

#[test]
fn a_stream_lives_on_its_keepalives_and_ends_with_its_connection() {
    let limits = ConnectionLimits {
        lifetime: Duration::from_millis(1_500),
        ping: Duration::from_millis(200),
        ..ConnectionLimits::PRODUCTION
    };
    let running = start_with(SERVER_UID, 0, limits);
    let began = Instant::now();
    let answer = whole(&running.socket, STREAM);
    let took = began.elapsed();
    let head = answer
        .split("\r\n\r\n")
        .next()
        .unwrap()
        .to_ascii_lowercase();
    assert!(head.starts_with("http/1.1 200"), "{answer}");
    for header in [
        "content-type: text/event-stream",
        "transfer-encoding: chunked",
        "connection: close",
        "cache-control: no-store",
    ] {
        assert!(head.contains(header), "{header}: {head}");
    }
    assert!(answer.contains("hello\n"), "{answer}");
    assert!(answer.matches(": ping").count() >= 4, "{answer}");
    assert!(
        answer.ends_with("0\r\n\r\n"),
        "the stream ends whole: {answer:?}"
    );
    assert!(
        took >= Duration::from_millis(1_400) && took < Duration::from_secs(5),
        "{took:?}"
    );
    assert_eq!(
        running.handler.streams.load(Ordering::SeqCst),
        0,
        "its source is let go"
    );
}

/// The head timeout (10 s in production, 500 ms here), which ends an idle
/// keep-alive, never ends a healthy stream: hyper reads no next head while
/// a reply is sent.
#[test]
fn a_stream_outlives_the_head_timeout() {
    let limits = ConnectionLimits {
        lifetime: Duration::from_secs(30),
        ping: Duration::from_millis(200),
        head: Duration::from_millis(500),
        ..ConnectionLimits::PRODUCTION
    };
    let running = start_with(SERVER_UID, 0, limits);
    let mut stream = unix_stream(&running.socket, STREAM);
    let (seen, closed) = read_for(&mut stream, Duration::from_millis(1_500));
    assert!(!closed, "still open at three head timeouts: {seen}");
    assert!(seen.matches(": ping").count() >= 5, "{seen}");
}

/// Review (#2756 M): a reader that goes gives back its stream's places at
/// once, not at the next keep-alive: a token that drops both its streams
/// opens a new one within a second.
#[test]
fn a_reader_that_goes_gives_its_places_back_at_once() {
    let running = start_tcp_with(SERVER_UID, ConnectionLimits::PRODUCTION);
    let address = running.tcp.unwrap();
    let held: Vec<_> = (0..MAX_STREAMS_PER_TOKEN)
        .map(|_| tcp_stream(address, "Bearer a"))
        .collect();
    for (_, seen) in &held {
        assert!(seen.starts_with("HTTP/1.1 200"), "{seen}");
    }
    assert_eq!(running.handler.streams.load(Ordering::SeqCst), 2);
    drop(held);
    let began = Instant::now();
    while running.handler.streams.load(Ordering::SeqCst) > 0
        && began.elapsed() < Duration::from_secs(1)
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    let (_again, seen) = tcp_stream(address, "Bearer a");
    assert!(seen.starts_with("HTTP/1.1 200"), "reopened at once: {seen}");
    assert!(
        began.elapsed() < Duration::from_secs(2),
        "{:?}",
        began.elapsed()
    );
}

/// A reader that takes nothing never holds the server: its writes stall,
/// the write deadline drops the connection, and the stream's source with
/// it, while other requests are answered.
#[test]
fn a_stalled_reader_is_let_go() {
    let limits = ConnectionLimits {
        write_timeout: Duration::from_millis(300),
        ..ConnectionLimits::PRODUCTION
    };
    let running = start_with(SERVER_UID, 0, limits);
    let stalled = unix_stream(
        &running.socket,
        b"GET /v1/stream?flood HTTP/1.1\r\nHost: board\r\n\r\n",
    );
    let began = Instant::now();
    while running.handler.streams.load(Ordering::SeqCst) == 0
        && began.elapsed() < Duration::from_secs(5)
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    let thing = b"GET /v1/thing HTTP/1.1\r\nHost: board\r\nConnection: close\r\n\r\n";
    assert_eq!(
        raw(&running.socket, thing).0,
        200,
        "served beside a stalled stream"
    );
    while running.handler.streams.load(Ordering::SeqCst) > 0
        && began.elapsed() < Duration::from_secs(5)
    {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        running.handler.streams.load(Ordering::SeqCst),
        0,
        "the stalled stream is dropped"
    );
    assert!(began.elapsed() < Duration::from_secs(5));
    drop(stalled);
}

fn tcp_stream(address: std::net::SocketAddr, bearer: &str) -> (TcpStream, String) {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    let request =
        format!("GET /v1/stream HTTP/1.1\r\nHost: board\r\nAuthorization: {bearer}\r\n\r\n");
    stream.write_all(request.as_bytes()).unwrap();
    let (seen, _) = read_head(&mut stream);
    (stream, seen)
}

/// A token holds at most 2 streams (each also one of its 8 connections);
/// another is 503 `busy`, while another token streams.
#[test]
fn a_token_holds_at_most_two_streams() {
    let running = start_tcp_with(SERVER_UID, ConnectionLimits::PRODUCTION);
    let address = running.tcp.unwrap();
    let held: Vec<_> = (0..MAX_STREAMS_PER_TOKEN)
        .map(|_| tcp_stream(address, "Bearer a"))
        .collect();
    for (_, seen) in &held {
        assert!(seen.starts_with("HTTP/1.1 200"), "{seen}");
    }
    let (_, refused) = tcp_stream(address, "Bearer a");
    assert!(refused.starts_with("HTTP/1.1 503"), "{refused}");
    assert!(refused.contains(r#""code":"busy""#), "{refused}");
    let (_other, seen) = tcp_stream(address, "Bearer b");
    assert!(
        seen.starts_with("HTTP/1.1 200"),
        "another token streams: {seen}"
    );
}

/// The server holds at most 32 streams at once, the socket's among them.
#[test]
fn the_server_holds_at_most_32_streams() {
    let running = start_with(SERVER_UID, 0, ConnectionLimits::PRODUCTION);
    let held: Vec<_> = (0..MAX_STREAMS)
        .map(|_| {
            let mut stream = unix_stream(&running.socket, STREAM);
            let (seen, _) = read_head(&mut stream);
            assert!(seen.starts_with("HTTP/1.1 200"), "{seen}");
            stream
        })
        .collect();
    let mut refused = unix_stream(&running.socket, STREAM);
    let (seen, _) = read_head(&mut refused);
    assert!(seen.starts_with("HTTP/1.1 503"), "{seen}");
    drop(held);
}
