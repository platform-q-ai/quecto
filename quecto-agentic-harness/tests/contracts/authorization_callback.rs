//! Contract for the auth capability's `AuthorizationCallback` (#1839): the
//! loopback listener on the provider's redirect URI returns the code of
//! the callback carrying the login's state, keeps waiting past one that
//! does not, reports an OAuth error callback as a failure the user may
//! paste past, gives up at its deadline, and refuses a redirect URI it must
//! not listen on. Only the test itself calls the listener; no provider is
//! contacted.
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use quecto::application::auth::ports::{AuthorizationCallback, AuthorizationCallbackError};
use quecto::domain::auth::provider::LoginProvider;
use quecto::infrastructure::auth::browser_login::OAuthBrowserLogin;
use quecto::infrastructure::auth::oauth::OAuthEndpoints;

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// The listener's answers, head and body as it writes them.
const SUCCESS_HEAD: &str = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n";
const SUCCESS_MARK: &str = "Authentication successful!";
const STATE_MISMATCH: &str =
    "HTTP/1.1 400 Bad Request\r\nConnection: close\r\nContent-Length: 14\r\n\r\nState mismatch";
const AUTH_FAILURE: &str =
    "HTTP/1.1 400 Bad Request\r\nConnection: close\r\nContent-Length: 12\r\n\r\nAuth failure";

/// Whether `answer` is the listener's success page.
fn succeeded(answer: &Option<String>) -> bool {
    answer
        .as_deref()
        .is_some_and(|answer| answer.starts_with(SUCCESS_HEAD) && answer.contains(SUCCESS_MARK))
}

/// Whether `answer` came from the listener at all.
fn from_the_listener(answer: &Option<String>) -> bool {
    succeeded(answer) || matches!(answer.as_deref(), Some(STATE_MISMATCH | AUTH_FAILURE))
}

/// Request `path` from whatever accepts on `port`, within bounds: up to
/// 10 s to connect, 10 s to read, the connection closed after one answer.
/// `None` when nothing answered in time.
fn call(port: u16, path: &str) -> Option<String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut stream = loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) => break stream,
            Err(_) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => return None,
        }
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .ok()?;
    stream
        .write_all(
            format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .ok()?;
    let mut answer = String::new();
    match stream.read_to_string(&mut answer) {
        Ok(_) => Some(answer),
        Err(_) if !answer.is_empty() => Some(answer),
        Err(_) => None,
    }
}

/// How long a listener under test waits: long enough for a loaded
/// runner, short enough that a lost port race costs seconds, not the
/// production 300.
const TEST_CALLBACK_SECS: u64 = 15;

/// Whether an attempt lost a port race rather than met the listener: it
/// could not bind, or it timed out having answered none of the calls.
fn lost_a_port_race(
    received: &Result<String, AuthorizationCallbackError>,
    answers: &[Option<String>],
) -> bool {
    match received {
        Err(AuthorizationCallbackError::Failed(why)) if why.contains("failed to bind") => true,
        Err(AuthorizationCallbackError::Failed(why)) if why.contains("timed out") => {
            !answers.iter().any(from_the_listener)
        }
        _ => false,
    }
}

/// The xAI listener on a test server's loopback redirect, receiving on a
/// thread while `calls` run against it: what it received and every answer
/// the calls got. A free port can be taken by another test's server
/// between its release and the listener's bind; only then is the attempt
/// repeated on a fresh port.
fn receive_while(
    calls: impl Fn(u16) -> Vec<Option<String>> + Send + Sync + Clone + 'static,
) -> (
    Result<String, AuthorizationCallbackError>,
    Vec<Option<String>>,
) {
    let mut last = None;
    for _attempt in 0..3 {
        let port = free_port();
        let login = OAuthBrowserLogin::with_callback_timeout_secs(
            OAuthEndpoints::new(Some(format!("http://127.0.0.1:{port}"))),
            TEST_CALLBACK_SECS,
        );
        let caller = {
            let calls = calls.clone();
            std::thread::spawn(move || calls(port))
        };
        let received = login.receive(LoginProvider::Xai, "fake-state");
        let answers = caller.join().unwrap();
        if !lost_a_port_race(&received, &answers) {
            return (received, answers);
        }
        last = Some((received, answers));
    }
    panic!("three port races in a row; last attempt: {last:?}");
}

#[test]
fn the_callback_carrying_the_state_returns_its_code() {
    let (received, answers) =
        receive_while(|port| vec![call(port, "/callback?code=fake-code&state=fake-state")]);
    assert!(succeeded(&answers[0]), "{answers:?} / {received:?}");
    assert_eq!(received, Ok("fake-code".to_string()), "{answers:?}");
}

#[test]
fn a_callback_with_another_state_is_refused_and_waiting_goes_on() {
    let (received, answers) = receive_while(|port| {
        let refused = call(port, "/callback?code=forged&state=other");
        let accepted = call(port, "/callback?code=fake-code&state=fake-state");
        vec![refused, accepted]
    });
    assert_eq!(
        answers[0].as_deref(),
        Some(STATE_MISMATCH),
        "{answers:?} / {received:?}"
    );
    assert!(succeeded(&answers[1]), "{answers:?} / {received:?}");
    assert_eq!(received, Ok("fake-code".to_string()), "{answers:?}");
}

#[test]
fn an_oauth_error_callback_is_a_failure_to_paste_past() {
    let (received, answers) =
        receive_while(|port| vec![call(port, "/callback?error=access_denied&state=fake-state")]);
    assert_eq!(
        answers[0].as_deref(),
        Some(AUTH_FAILURE),
        "{answers:?} / {received:?}"
    );
    assert!(
        matches!(received, Err(AuthorizationCallbackError::Failed(_))),
        "{received:?} / {answers:?}"
    );
}

#[test]
fn a_redirect_off_this_machine_is_never_listened_on() {
    let login =
        OAuthBrowserLogin::new(OAuthEndpoints::new(Some("http://example.test".to_string())));
    assert!(matches!(
        login.receive(LoginProvider::Xai, "fake-state"),
        Err(AuthorizationCallbackError::InvalidRedirect(_))
    ));
}

#[test]
fn a_provider_without_a_callback_fails_at_once() {
    let login = OAuthBrowserLogin::new(OAuthEndpoints::new(None));
    assert!(matches!(
        login.receive(LoginProvider::Anthropic, "fake-state"),
        Err(AuthorizationCallbackError::Failed(_))
    ));
}

#[test]
fn a_callback_that_never_comes_fails_at_the_deadline() {
    let port = free_port();
    let login = OAuthBrowserLogin::with_callback_timeout_secs(
        OAuthEndpoints::new(Some(format!("http://127.0.0.1:{port}"))),
        1,
    );
    let started = std::time::Instant::now();
    assert_eq!(
        login.receive(LoginProvider::Xai, "fake-state"),
        Err(AuthorizationCallbackError::Failed(
            "provider error: OAuth callback timed out".to_string()
        ))
    );
    assert!(started.elapsed() < Duration::from_secs(10));
}
