// Tests for the OAuth callback listener's connection handling (PR #2309).

use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

fn later(secs: u64) -> Instant {
    Instant::now() + Duration::from_secs(secs)
}

const LIMITS: CallbackLimits = CallbackLimits {
    connection_budget: Duration::from_millis(200),
    linger: Duration::from_millis(200),
    ..CallbackLimits::PRODUCTION
};

/// A server end whose client has already sent `bytes`; the client end is
/// returned so the test decides whether it stays open.
async fn sent(bytes: &[u8]) -> (DuplexStream, DuplexStream) {
    let (mut client, server) = tokio::io::duplex(1024 * 1024);
    client.write_all(bytes).await.unwrap();
    (client, server)
}

/// Content-Length of a canned response must match its body exactly.
fn assert_content_length_matches(response: &str) {
    let (head, body) = response.split_once("\r\n\r\n").expect("head and body");
    let declared: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length: "))
        .expect("Content-Length header")
        .parse()
        .expect("numeric Content-Length");
    assert_eq!(declared, body.len(), "{response:?}");
}

#[test]
fn canned_responses_declare_their_body_length() {
    assert_content_length_matches(LINE_TOO_LONG_RESPONSE);
    assert_content_length_matches(HEAD_TOO_LARGE_RESPONSE);
    assert_content_length_matches(INCOMPLETE_REQUEST_RESPONSE);
    assert!(LINE_TOO_LONG_RESPONSE.starts_with("HTTP/1.1 414 "));
    assert!(HEAD_TOO_LARGE_RESPONSE.starts_with("HTTP/1.1 431 "));
    assert!(INCOMPLETE_REQUEST_RESPONSE.starts_with("HTTP/1.1 400 "));
}

#[test]
fn production_limits_fit_a_real_callback_and_stay_bounded() {
    let limits = CallbackLimits::PRODUCTION;
    assert!(limits.request_line_bytes >= 8 * 1024);
    assert!(limits.head_bytes >= limits.request_line_bytes);
    assert!(limits.connection_budget <= Duration::from_secs(10));
    assert!(limits.linger <= limits.connection_budget);
}

#[tokio::test]
async fn request_line_ends_at_crlf_or_bare_lf_without_its_ending() {
    for (input, rest) in [
        (
            &b"GET /cb?a=1 HTTP/1.1\r\nHost: x\r\n\r\n"[..],
            &b"Host: x\r\n\r\n"[..],
        ),
        (b"GET /cb?a=1 HTTP/1.1\nHost: x\n\n", b"Host: x\n\n"),
    ] {
        let (_client, server) = sent(input).await;
        let mut connection = CallbackConnection::new(server, &LIMITS, later(5));
        assert_eq!(
            connection.request_line().await,
            Framed::Ready("GET /cb?a=1 HTTP/1.1".to_string())
        );
        assert_eq!(connection.pending, rest);
    }
}

#[tokio::test]
async fn request_line_split_across_reads_is_read_whole() {
    let (mut client, server) = tokio::io::duplex(64);
    let writer = tokio::spawn(async move {
        for piece in [&b"GET /c"[..], b"b HTTP/1.1\r", b"\n"] {
            client.write_all(piece).await.unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        client
    });
    let mut connection = CallbackConnection::new(server, &LIMITS, later(5));
    assert_eq!(
        connection.request_line().await,
        Framed::Ready("GET /cb HTTP/1.1".to_string())
    );
    drop(writer.await.unwrap());
}

#[tokio::test]
async fn request_line_of_exactly_the_cap_is_accepted_and_one_byte_more_is_not() {
    let cap = LIMITS.request_line_bytes;
    let at_cap = format!("GET /{} HTTP/1.1\r\n", "p".repeat(cap - 16));
    assert_eq!(at_cap.len(), cap);
    let (_client, server) = sent(at_cap.as_bytes()).await;
    let mut connection = CallbackConnection::new(server, &LIMITS, later(5));
    assert_eq!(
        connection.request_line().await,
        Framed::Ready(at_cap.trim_end().to_string())
    );

    let over_cap = format!("GET /{} HTTP/1.1\r\n", "p".repeat(cap - 15));
    let (_client, server) = sent(over_cap.as_bytes()).await;
    let mut connection = CallbackConnection::new(server, &LIMITS, later(5));
    assert_eq!(
        connection.request_line().await,
        Framed::Refused(LINE_TOO_LONG_RESPONSE)
    );
}

#[tokio::test]
async fn endless_request_line_is_refused_shortly_past_the_cap() {
    let endless = vec![b'x'; LIMITS.request_line_bytes * 4];
    let (_client, server) = sent(&endless).await;
    let mut connection = CallbackConnection::new(server, &LIMITS, later(5));
    assert_eq!(
        connection.request_line().await,
        Framed::Refused(LINE_TOO_LONG_RESPONSE)
    );
    assert!(connection.head_read < LIMITS.request_line_bytes + READ_CHUNK_BYTES);
}

#[tokio::test]
async fn end_of_stream_before_the_line_ends_is_incomplete() {
    for partial in [&b""[..], b"GET ", b"GET /cb HTTP/1.1\r"] {
        let (client, server) = sent(partial).await;
        drop(client);
        let mut connection = CallbackConnection::new(server, &LIMITS, later(5));
        assert_eq!(
            connection.request_line().await,
            Framed::Refused(INCOMPLETE_REQUEST_RESPONSE),
            "{partial:?}"
        );
    }
}

#[tokio::test]
async fn silent_client_spends_its_budget_but_not_the_login() {
    let (_client, server) = sent(b"GET / HT").await;
    let start = Instant::now();
    let mut connection = CallbackConnection::new(server, &LIMITS, later(5));
    assert_eq!(
        connection.request_line().await,
        Framed::Refused(INCOMPLETE_REQUEST_RESPONSE)
    );
    let waited = start.elapsed();
    assert!(waited >= LIMITS.connection_budget, "{waited:?}");
    assert!(waited < Duration::from_secs(2), "{waited:?}");
}

#[tokio::test]
async fn silent_client_at_the_login_deadline_ends_the_login() {
    let (_client, server) = sent(b"").await;
    let deadline = Instant::now() + Duration::from_millis(50);
    let mut connection = CallbackConnection::new(server, &LIMITS, deadline);
    assert_eq!(connection.request_line().await, Framed::LoginTimedOut);
}

async fn head_after_line(input: &[u8], limits: &CallbackLimits, close: bool) -> Framed<()> {
    let (client, server) = sent(input).await;
    let _client = match close {
        true => None,
        false => Some(client),
    };
    let mut connection = CallbackConnection::new(server, limits, later(5));
    match connection.request_line().await {
        Framed::Ready(_) => connection.rest_of_head().await,
        Framed::Refused(response) => Framed::Refused(response),
        Framed::LoginTimedOut => Framed::LoginTimedOut,
    }
}

#[tokio::test]
async fn rest_of_head_ends_at_an_empty_line_of_either_ending() {
    for input in [
        &b"GET / HTTP/1.1\r\n\r\n"[..],
        b"GET / HTTP/1.1\n\n",
        b"GET / HTTP/1.1\r\nHost: x\r\nCookie: a=b\r\n\r\nbody",
        b"GET / HTTP/1.1\nHost: x\nCookie: a=b\n\nbody",
        b"GET / HTTP/1.1\r\nHost: x\n\r\n",
    ] {
        assert_eq!(
            head_after_line(input, &LIMITS, false).await,
            Framed::Ready(()),
            "{input:?}"
        );
    }
}

#[tokio::test]
async fn rest_of_head_blank_line_split_across_reads_is_found() {
    let (mut client, server) = tokio::io::duplex(64);
    let writer = tokio::spawn(async move {
        for piece in [&b"GET / HTTP/1.1\r\nHost: x\r"[..], b"\n\r", b"\n"] {
            client.write_all(piece).await.unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        client
    });
    let mut connection = CallbackConnection::new(server, &LIMITS, later(5));
    assert!(matches!(connection.request_line().await, Framed::Ready(_)));
    assert_eq!(connection.rest_of_head().await, Framed::Ready(()));
    drop(writer.await.unwrap());
}

#[tokio::test]
async fn rest_of_head_larger_than_the_line_cap_is_read_and_discarded() {
    let cookie = "c".repeat(100 * 1024);
    let input = format!("GET / HTTP/1.1\r\nCookie: a={cookie}\r\n\r\n");
    assert_eq!(
        head_after_line(input.as_bytes(), &LIMITS, false).await,
        Framed::Ready(())
    );
}

#[tokio::test]
async fn rest_of_head_past_the_head_cap_is_refused() {
    let limits = CallbackLimits {
        head_bytes: 16 * 1024,
        ..LIMITS
    };
    let input = format!("GET / HTTP/1.1\r\nX-Pad: {}\r\n\r\n", "p".repeat(20 * 1024));
    assert_eq!(
        head_after_line(input.as_bytes(), &limits, false).await,
        Framed::Refused(HEAD_TOO_LARGE_RESPONSE)
    );
}

/// A stream whose reads return the scripted sizes, in order, then as much as
/// the caller's buffer takes: the tests decide where each read ends, which a
/// socket never promises. Writes are accepted and discarded.
struct ScriptedReads {
    data: Vec<u8>,
    sizes: std::collections::VecDeque<usize>,
}

impl ScriptedReads {
    fn new(data: Vec<u8>, sizes: &[usize]) -> Self {
        Self {
            data,
            sizes: sizes.iter().copied().collect(),
        }
    }
}

impl tokio::io::AsyncRead for ScriptedReads {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let scripted = self.sizes.pop_front().unwrap_or(usize::MAX);
        let n = scripted.min(buf.remaining()).min(self.data.len());
        let rest = self.data.split_off(n);
        buf.put_slice(&self.data);
        self.data = rest;
        std::task::Poll::Ready(Ok(()))
    }
}

impl tokio::io::AsyncWrite for ScriptedReads {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::task::Poll::Ready(Ok(buf.len()))
    }
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}

/// A valid callback head of exactly `len` bytes: its final LF is byte `len`.
fn callback_head_of(len: usize) -> Vec<u8> {
    let line = "GET /callback?code=ok&state=s HTTP/1.1\r\n";
    let frame = line.len() + "X: ".len() + "\r\n\r\n".len();
    assert!(len > frame, "a head of {len} bytes cannot hold the line");
    let head = format!("{line}X: {}\r\n\r\n", "p".repeat(len - frame));
    assert_eq!(head.len(), len);
    head.into_bytes()
}

/// Frame a head read in the scripted sizes; also return how many head bytes
/// the connection read.
async fn scripted_head(head: Vec<u8>, sizes: &[usize]) -> (Framed<()>, usize) {
    let stream = ScriptedReads::new(head, sizes);
    let mut connection = CallbackConnection::new(stream, &CallbackLimits::PRODUCTION, later(5));
    let framed = match connection.request_line().await {
        Framed::Ready(_) => connection.rest_of_head().await,
        Framed::Refused(response) => Framed::Refused(response),
        Framed::LoginTimedOut => Framed::LoginTimedOut,
    };
    (framed, connection.head_read)
}

/// Read splits that move where the cap falls within a read: a short first
/// read (47 bytes, the swarm review's case), reads of one byte and of a
/// chunk less one, and whole chunks.
const SPLITS: [&[usize]; 4] = [&[47], &[1, 46, 4095], &[4096], &[40, 7, 1, 1]];

#[tokio::test]
async fn a_short_first_read_cannot_carry_a_terminator_past_the_head_cap() {
    let cap = CallbackLimits::PRODUCTION.head_bytes;
    assert_eq!(cap, 262_144);
    let (framed, head_read) = scripted_head(callback_head_of(262_154), &[47]).await;
    assert_eq!(framed, Framed::Refused(HEAD_TOO_LARGE_RESPONSE));
    assert!(
        head_read <= cap,
        "read {head_read} head bytes past a cap of {cap}"
    );
}

#[tokio::test]
async fn a_head_ending_exactly_at_the_cap_is_accepted_whatever_the_reads() {
    let cap = CallbackLimits::PRODUCTION.head_bytes;
    for sizes in SPLITS {
        let (framed, head_read) = scripted_head(callback_head_of(cap), sizes).await;
        assert_eq!(framed, Framed::Ready(()), "reads {sizes:?}");
        assert_eq!(head_read, cap, "reads {sizes:?}");
    }
}

#[tokio::test]
async fn a_head_ending_one_byte_past_the_cap_is_refused_whatever_the_reads() {
    let cap = CallbackLimits::PRODUCTION.head_bytes;
    for sizes in SPLITS {
        let (framed, head_read) = scripted_head(callback_head_of(cap + 1), sizes).await;
        assert_eq!(
            framed,
            Framed::Refused(HEAD_TOO_LARGE_RESPONSE),
            "reads {sizes:?}"
        );
        assert!(head_read <= cap, "reads {sizes:?}: read {head_read}");
    }
}

#[tokio::test]
async fn rest_of_head_cut_off_or_stalled_is_incomplete() {
    let input = b"GET / HTTP/1.1\r\nHost: x\r\n";
    for close in [true, false] {
        assert_eq!(
            head_after_line(input, &LIMITS, close).await,
            Framed::Refused(INCOMPLETE_REQUEST_RESPONSE),
            "close: {close}"
        );
    }
}

#[tokio::test]
async fn drain_stops_at_its_byte_limit() {
    let input = vec![b'b'; 10 * 4096 + 7];
    let mut reader = input.as_slice();
    assert_eq!(
        drain(&mut reader, later(5), 3 * 4096 + 5).await,
        3 * 4096 + 5
    );
    assert_eq!(reader.len(), input.len() - (3 * 4096 + 5));
}

#[tokio::test]
async fn drain_stops_at_end_of_stream() {
    let mut reader: &[u8] = b"short";
    assert_eq!(drain(&mut reader, later(5), 1024).await, 5);
}

#[tokio::test]
async fn drain_stops_when_its_time_is_up() {
    let (_client, mut server) = tokio::io::duplex(64);
    let start = Instant::now();
    let until = start + Duration::from_millis(100);
    assert_eq!(drain(&mut server, until, 1024).await, 0);
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn answer_writes_the_response_half_closes_and_drains() {
    let (mut client, server) = sent(b"GET / HTTP/1.1\r\n\r\nleftover body").await;
    let connection = CallbackConnection::new(server, &LIMITS, later(5));
    let answering = tokio::spawn(connection.answer(INCOMPLETE_REQUEST_RESPONSE));
    let mut answer = String::new();
    client.read_to_string(&mut answer).await.unwrap();
    assert_eq!(answer, INCOMPLETE_REQUEST_RESPONSE);
    tokio::time::timeout(Duration::from_secs(2), answering)
        .await
        .expect("the linger bounds the drain of a client that stays open")
        .unwrap();
}

#[test]
fn head_scan_ends_only_at_an_empty_line() {
    let scan = |bytes: &[u8]| {
        bytes
            .iter()
            .fold(HeadScan::AtLineStart, |state, byte| state.next(*byte))
    };
    assert_eq!(scan(b"\n"), HeadScan::Ended);
    assert_eq!(scan(b"\r\n"), HeadScan::Ended);
    assert_eq!(scan(b"a: b\r\n"), HeadScan::AtLineStart);
    assert_eq!(scan(b"a: b\r\n\r"), HeadScan::AfterLineStartCr);
    assert_eq!(scan(b"a: b\r\n\rx"), HeadScan::InLine);
    assert_eq!(scan(b"a\r\rb"), HeadScan::InLine);
    assert_eq!(scan(b"\n\nanything"), HeadScan::Ended);
}
