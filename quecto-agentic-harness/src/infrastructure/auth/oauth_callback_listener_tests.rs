// Tests for the OAuth callback listener's connection handling (PR #2309).
//
// The listener frames and caps only the request line; a stray or broken
// connection is answered and dropped within its own budget, never ending the
// login; and every answer is followed by a bounded half-close-and-drain.

use super::*;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Limits small enough that a test waiting one of them out stays fast.
fn fast_limits() -> CallbackLimits {
    CallbackLimits {
        connection_budget: Duration::from_millis(300),
        linger: Duration::from_millis(300),
        ..CallbackLimits::PRODUCTION
    }
}

/// Bind an ephemeral port and serve one login on `/callback` with state `s`.
async fn start(
    login: Duration,
    limits: CallbackLimits,
) -> (
    SocketAddr,
    tokio::task::JoinHandle<Result<String, DomainError>>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let deadline = Instant::now() + login;
    let handle = tokio::spawn(async move {
        serve_oauth_callback(listener, "/callback", "s", deadline, &limits).await
    });
    (addr, handle)
}

/// Connect and send `bytes`, keeping the connection open and unread.
async fn hold_open(addr: SocketAddr, bytes: &[u8]) -> TcpStream {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.write_all(bytes).await.unwrap();
    stream
}

/// Connect, send `bytes` in one write, and return what the listener answered.
/// I/O errors and a missing answer are folded into the returned text so the
/// test fails on its assertion, naming the cause.
async fn exchange(addr: SocketAddr, bytes: &[u8]) -> String {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    if let Err(error) = stream.write_all(bytes).await {
        return format!("write error: {error}");
    }
    let mut buf = Vec::new();
    let read = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut buf)).await;
    match read {
        Ok(Ok(_)) => String::from_utf8_lossy(&buf).into_owned(),
        Ok(Err(error)) => format!(
            "read error: {error}; partial: {:?}",
            String::from_utf8_lossy(&buf)
        ),
        Err(_) => "read timed out: the listener never answered".to_string(),
    }
}

const VALID: &[u8] = b"GET /callback?code=ok&state=s HTTP/1.1\r\nHost: x\r\n\r\n";

#[tokio::test]
async fn a_valid_line_with_a_large_cookie_header_is_served() {
    // Cookies for localhost are shared across ports, so a browser's head can
    // be far larger than the request line the listener needs.
    let (addr, handle) = start(Duration::from_secs(10), CallbackLimits::PRODUCTION).await;
    let cookie = "c".repeat(19 * 1024 + 512);
    let request =
        format!("GET /callback?code=ok&state=s HTTP/1.1\r\nHost: x\r\nCookie: a={cookie}\r\n\r\n");
    let resp = exchange(addr, request.as_bytes()).await;
    assert!(resp.starts_with("HTTP/1.1 200 OK"), "got: {resp:?}");
    assert_eq!(handle.await.unwrap().unwrap(), "ok");
}

#[tokio::test]
async fn a_stray_partial_head_does_not_hold_the_listener() {
    let (addr, handle) = start(Duration::from_secs(10), fast_limits()).await;
    let start_at = std::time::Instant::now();
    let _stray = hold_open(addr, b"GET / HTTP/1.1\r\n").await;
    let resp = exchange(addr, VALID).await;
    assert!(resp.starts_with("HTTP/1.1 200 OK"), "got: {resp:?}");
    assert_eq!(handle.await.unwrap().unwrap(), "ok");
    assert!(
        start_at.elapsed() < Duration::from_secs(3),
        "{:?}",
        start_at.elapsed()
    );
}

#[tokio::test]
async fn a_stray_silent_connection_does_not_hold_the_listener() {
    let (addr, handle) = start(Duration::from_secs(10), fast_limits()).await;
    let start_at = std::time::Instant::now();
    let _stray = hold_open(addr, b"").await;
    let resp = exchange(addr, VALID).await;
    assert!(resp.starts_with("HTTP/1.1 200 OK"), "got: {resp:?}");
    assert_eq!(handle.await.unwrap().unwrap(), "ok");
    assert!(
        start_at.elapsed() < Duration::from_secs(3),
        "{:?}",
        start_at.elapsed()
    );
}

#[tokio::test]
async fn a_bare_lf_request_is_served() {
    let (addr, handle) = start(Duration::from_secs(3), fast_limits()).await;
    let resp = exchange(addr, b"GET /callback?code=ok&state=s HTTP/1.1\nHost: x\n\n").await;
    assert!(resp.starts_with("HTTP/1.1 200 OK"), "got: {resp:?}");
    assert_eq!(handle.await.unwrap().unwrap(), "ok");
}

#[tokio::test]
async fn an_over_long_request_line_is_refused_and_the_listener_keeps_serving() {
    let (addr, handle) = start(Duration::from_secs(10), fast_limits()).await;
    let code = "c".repeat(CallbackLimits::PRODUCTION.request_line_bytes);
    let request = format!("GET /callback?code={code}&state=s HTTP/1.1\r\nHost: x\r\n\r\n");
    let resp = exchange(addr, request.as_bytes()).await;
    assert!(resp.starts_with("HTTP/1.1 414 "), "got: {resp:?}");
    let resp = exchange(addr, VALID).await;
    assert!(resp.starts_with("HTTP/1.1 200 OK"), "got: {resp:?}");
    assert_eq!(handle.await.unwrap().unwrap(), "ok");
}

#[tokio::test]
async fn a_silent_answered_client_delays_the_next_callback_by_at_most_the_linger() {
    let limits = CallbackLimits {
        linger: Duration::from_millis(400),
        connection_budget: Duration::from_secs(5),
        ..CallbackLimits::PRODUCTION
    };
    let (addr, handle) = start(Duration::from_secs(10), limits).await;
    let start_at = std::time::Instant::now();
    let _answered = hold_open(addr, b"GET /elsewhere HTTP/1.1\r\nHost: x\r\n\r\n").await;
    let resp = exchange(addr, VALID).await;
    assert!(resp.starts_with("HTTP/1.1 200 OK"), "got: {resp:?}");
    assert_eq!(handle.await.unwrap().unwrap(), "ok");
    // 400 ms of linger plus a generous scheduling margin.
    assert!(
        start_at.elapsed() < Duration::from_millis(1400),
        "{:?}",
        start_at.elapsed()
    );
}

#[tokio::test]
async fn lingering_never_runs_past_the_login_deadline() {
    let limits = CallbackLimits {
        linger: Duration::from_secs(30),
        connection_budget: Duration::from_secs(30),
        ..CallbackLimits::PRODUCTION
    };
    let (addr, handle) = start(Duration::from_millis(500), limits).await;
    let start_at = std::time::Instant::now();
    let long_line = format!("GET /{} HTTP/1.1\r\n", "x".repeat(20 * 1024));
    let _refused = hold_open(addr, long_line.as_bytes()).await;
    let err = handle.await.unwrap().unwrap_err();
    assert!(err.to_string().contains("timed out"), "got: {err}");
    assert!(
        start_at.elapsed() < Duration::from_millis(1500),
        "{:?}",
        start_at.elapsed()
    );
}

/// A request followed by bytes the answer does not need: the listener must
/// drain them before closing, or the kernel resets the connection and the
/// client may lose the answer.
fn with_body(line: &str) -> Vec<u8> {
    let body = vec![b'b'; 32 * 1024];
    let mut request = format!(
        "{line}\r\nHost: x\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes();
    request.extend_from_slice(&body);
    request
}

#[tokio::test]
async fn a_not_found_answer_with_unread_body_arrives_whole() {
    let (addr, handle) = start(Duration::from_secs(10), fast_limits()).await;
    let resp = exchange(addr, &with_body("POST /elsewhere HTTP/1.1")).await;
    assert!(
        resp.starts_with("HTTP/1.1 404 ") && resp.ends_with("Not found"),
        "got: {resp:?}"
    );
    let resp = exchange(addr, VALID).await;
    assert!(resp.starts_with("HTTP/1.1 200 OK"), "got: {resp:?}");
    assert_eq!(handle.await.unwrap().unwrap(), "ok");
}

#[tokio::test]
async fn a_success_answer_with_unread_body_arrives_whole() {
    let (addr, handle) = start(Duration::from_secs(10), fast_limits()).await;
    let resp = exchange(addr, &with_body("POST /callback?code=ok&state=s HTTP/1.1")).await;
    assert!(
        resp.starts_with("HTTP/1.1 200 OK") && resp.ends_with("</html>"),
        "got: {resp:?}"
    );
    assert_eq!(handle.await.unwrap().unwrap(), "ok");
}

#[test]
fn canned_route_responses_declare_their_body_length() {
    for response in [
        NOT_FOUND_RESPONSE,
        STATE_MISMATCH_RESPONSE,
        MISSING_CODE_RESPONSE,
        AUTH_FAILURE_RESPONSE,
    ] {
        let (head, body) = response.split_once("\r\n\r\n").expect("head and body");
        let declared = format!("Content-Length: {}", body.len());
        assert!(head.lines().any(|line| line == declared), "{response:?}");
    }
}

#[test]
fn route_decides_from_the_request_line_alone() {
    let route = |line: &str| route(line, "/callback", "s");
    assert_eq!(
        route("GET /callback?code=a%2Fb&state=s HTTP/1.1"),
        Route::Authorized("a/b".to_string())
    );
    assert_eq!(route("GET / HTTP/1.1"), Route::Declined(NOT_FOUND_RESPONSE));
    assert_eq!(
        route("GET /callbackevil?code=x&state=s HTTP/1.1"),
        Route::Declined(NOT_FOUND_RESPONSE)
    );
    assert_eq!(route(""), Route::Declined(NOT_FOUND_RESPONSE));
    assert_eq!(
        route("GET /callback?code=x&state=t HTTP/1.1"),
        Route::Declined(STATE_MISMATCH_RESPONSE)
    );
    assert_eq!(
        route("GET /callback?code=x HTTP/1.1"),
        Route::Declined(STATE_MISMATCH_RESPONSE)
    );
    assert_eq!(
        route("GET /callback?code=&state=s HTTP/1.1"),
        Route::Declined(MISSING_CODE_RESPONSE)
    );
    assert_eq!(
        route("GET /callback?state=s HTTP/1.1"),
        Route::Declined(MISSING_CODE_RESPONSE)
    );
    assert_eq!(
        route("GET /callback?error=access_denied%3Cx%3E&state=s HTTP/1.1"),
        Route::Denied("access_deniedx".to_string())
    );
    assert_eq!(
        route("GET /callback?error=%3C%3E&state=s HTTP/1.1"),
        Route::Denied("unknown_error".to_string())
    );
}

#[test]
fn constant_time_eq_matches_only_identical_strings() {
    assert!(constant_time_eq("", ""));
    assert!(constant_time_eq("abc", "abc"));
    assert!(!constant_time_eq("abc", "abd"));
    assert!(!constant_time_eq("abc", "ab"));
    assert!(!constant_time_eq("ab", "abc"));
    assert!(!constant_time_eq("", "a"));
}

#[tokio::test]
async fn a_valid_line_whose_head_is_cut_off_does_not_end_the_login() {
    let (addr, handle) = start(Duration::from_secs(10), fast_limits()).await;
    // The client stays open but never ends its head: the connection's budget
    // refuses it and the next callback is served.
    let _stalled = hold_open(addr, b"GET /callback?code=early&state=s HTTP/1.1\r\n").await;
    let resp = exchange(addr, VALID).await;
    assert!(resp.starts_with("HTTP/1.1 200 OK"), "got: {resp:?}");
    assert_eq!(handle.await.unwrap().unwrap(), "ok");
}

#[tokio::test]
async fn an_error_callback_whose_head_is_cut_off_does_not_end_the_login() {
    let (addr, handle) = start(Duration::from_secs(10), fast_limits()).await;
    // An OAuth error callback ends the login only once its whole head has
    // arrived: a stalled one is refused and the next callback is served.
    let _stalled = hold_open(addr, b"GET /callback?error=x&state=s HTTP/1.1\r\n").await;
    let resp = exchange(addr, VALID).await;
    assert!(resp.starts_with("HTTP/1.1 200 OK"), "got: {resp:?}");
    assert_eq!(handle.await.unwrap().unwrap(), "ok");
}

#[tokio::test]
async fn a_trickling_stray_is_cut_off_at_its_total_budget_not_per_read() {
    // Budget 300 ms, linger 100 ms. The stray sends one byte every 50 ms and
    // never ends its line: were the budget a per-read timeout, it would hold
    // the listener for as long as it kept trickling.
    let limits = CallbackLimits {
        connection_budget: Duration::from_millis(300),
        linger: Duration::from_millis(100),
        ..CallbackLimits::PRODUCTION
    };
    let (addr, handle) = start(Duration::from_secs(10), limits).await;
    let start_at = std::time::Instant::now();
    let mut stray = hold_open(addr, b"G").await;
    let trickle = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(50)).await;
            match stray.write_all(b"E").await {
                Ok(()) => {}
                Err(_) => return,
            }
        }
    });
    let resp = exchange(addr, VALID).await;
    trickle.abort();
    assert!(resp.starts_with("HTTP/1.1 200 OK"), "got: {resp:?}");
    assert_eq!(handle.await.unwrap().unwrap(), "ok");
    // 300 ms of budget plus 100 ms of linger plus a generous margin.
    assert!(
        start_at.elapsed() < Duration::from_millis(1500),
        "{:?}",
        start_at.elapsed()
    );
}

#[tokio::test]
async fn an_empty_expected_state_is_an_error_not_a_panic() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let err = serve_oauth_callback(listener, "/callback", "", deadline, &fast_limits())
        .await
        .expect_err("an empty expected state must be refused");
    assert!(
        err.to_string().contains("non-empty expected state"),
        "got: {err}"
    );
}
