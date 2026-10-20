//! Contract for the auth capability's `AuthorizationCallback` (#1839): the
//! loopback listener on the provider's redirect URI returns the code of
//! the callback carrying the login's state, keeps waiting past one that
//! does not, reports an OAuth error callback as a failure the user may
//! paste past, gives up at its deadline, and refuses a redirect URI it must
//! not listen on. Only the test itself calls the listener; no provider is
//! contacted. Every attempt has its own callback path and state, so tests
//! running concurrently can never feed each other's listeners.
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
const NOT_FOUND: &str =
    "HTTP/1.1 404 Not Found\r\nConnection: close\r\nContent-Length: 9\r\n\r\nNot found";
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
/// runner, short enough that a missed callback costs seconds, not the
/// production 300.
const TEST_CALLBACK_SECS: u64 = 15;

/// One listener's own address: a free port, and a callback path and state
/// no other attempt — in this process or a concurrent one — shares. Tests
/// run concurrently and a released port can be handed out twice, so two
/// listeners can meet on one port: with its own path and state, a listener
/// answers a foreign callback "404 Not found" and never accepts it, and an
/// attempt that reached a foreign listener knows it by that answer.
struct Attempt {
    port: u16,
    nonce: String,
}

impl Attempt {
    fn fresh() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self {
            port: free_port(),
            nonce: format!("t{}-{n}", std::process::id()),
        }
    }

    /// The login's redirect base: the listener serves `<base>/callback`.
    fn base(&self) -> String {
        format!("http://127.0.0.1:{}/{}", self.port, self.nonce)
    }

    /// The state this attempt's login is bound to.
    fn state(&self) -> String {
        format!("state-{}", self.nonce)
    }

    /// Call this attempt's callback with `query` (`{state}` is replaced by
    /// its state).
    fn call(&self, query: &str) -> Option<String> {
        let query = query.replace("{state}", &self.state());
        call(self.port, &format!("/{}/callback?{query}", self.nonce))
    }
}

/// Whether an attempt met another attempt's listener or a foreign server
/// instead of its own: its listener could not bind, or it timed out while
/// something else answered the calls (none of the answers its own). A
/// timeout with no answers at all is no evidence of a stranger: the
/// listener itself may be refusing its own path, which is asserted.
fn met_a_stranger(
    received: &Result<String, AuthorizationCallbackError>,
    answers: &[Option<String>],
) -> bool {
    match received {
        Err(AuthorizationCallbackError::Failed(why)) if why.contains("failed to bind") => true,
        Err(AuthorizationCallbackError::Failed(why)) if why.contains("timed out") => {
            answers.iter().any(Option::is_some) && !answers.iter().any(from_the_listener)
        }
        _ => false,
    }
}

/// The xAI listener on a test server's loopback redirect, receiving on a
/// thread while `calls` run against it: what it received and every answer
/// the calls got. Only an attempt that met a stranger (see [`Attempt`]) is
/// repeated, on a fresh port.
fn receive_while(
    calls: impl Fn(&Attempt) -> Vec<Option<String>> + Send + Sync + Clone + 'static,
) -> (
    Result<String, AuthorizationCallbackError>,
    Vec<Option<String>>,
) {
    let mut last = None;
    for _attempt in 0..3 {
        let attempt = std::sync::Arc::new(Attempt::fresh());
        let login = OAuthBrowserLogin::with_callback_timeout_secs(
            OAuthEndpoints::new(Some(attempt.base())),
            TEST_CALLBACK_SECS,
        );
        let caller = {
            let calls = calls.clone();
            let attempt = std::sync::Arc::clone(&attempt);
            std::thread::spawn(move || calls(&attempt))
        };
        let received = login.receive(LoginProvider::Xai, &attempt.state());
        let answers = caller.join().unwrap();
        if !met_a_stranger(&received, &answers) {
            return (received, answers);
        }
        last = Some((received, answers));
    }
    panic!(
        "three attempts in a row failed without meeting their listener: a stranger held \
         the port each time, or the listener refuses its own path; last: {last:?}"
    );
}

#[test]
fn the_callback_carrying_the_state_returns_its_code() {
    let (received, answers) =
        receive_while(|attempt| vec![attempt.call("code=fake-code&state={state}")]);
    assert!(succeeded(&answers[0]), "{answers:?} / {received:?}");
    assert_eq!(received, Ok("fake-code".to_string()), "{answers:?}");
}

#[test]
fn a_callback_with_another_state_is_refused_and_waiting_goes_on() {
    let (received, answers) = receive_while(|attempt| {
        let refused = attempt.call("code=forged&state=other");
        let accepted = attempt.call("code=fake-code&state={state}");
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
fn a_callback_on_another_path_is_not_found_and_waiting_goes_on() {
    // What keeps concurrent attempts apart: a listener takes only its own
    // path, so a callback meant for another listener on a reused port is
    // answered "404 Not found" and never accepted.
    let (received, answers) = receive_while(|attempt| {
        // Proof first that this attempt's own listener holds the port: only
        // it answers its own path's wrong state "State mismatch", and it
        // keeps the port until it returns. Until then the port may still be
        // a stranger's (a released port can be handed out twice), and a
        // stranger's answer to the foreign call proves nothing. Without the
        // proof the foreign and accepting calls are not made, so the listener
        // times out having met a stranger and the attempt is repeated.
        let proof = attempt.call("code=forged&state=other");
        if proof.as_deref() != Some(STATE_MISMATCH) {
            return vec![proof];
        }
        let foreign = call(
            attempt.port,
            &format!("/elsewhere/callback?code=foreign&state={}", attempt.state()),
        );
        let accepted = attempt.call("code=fake-code&state={state}");
        vec![proof, foreign, accepted]
    });
    assert_eq!(
        answers[0].as_deref(),
        Some(STATE_MISMATCH),
        "the attempt's own listener holds the port: {answers:?} / {received:?}"
    );
    assert_eq!(
        answers[1].as_deref(),
        Some(NOT_FOUND),
        "{answers:?} / {received:?}"
    );
    assert!(succeeded(&answers[2]), "{answers:?} / {received:?}");
    assert_eq!(received, Ok("fake-code".to_string()), "{answers:?}");
}

#[test]
fn an_oauth_error_callback_is_a_failure_to_paste_past() {
    let (received, answers) =
        receive_while(|attempt| vec![attempt.call("error=access_denied&state={state}")]);
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
    // Repeated only when the port was taken before the listener bound it.
    let received = (0..3)
        .map(|_| {
            let attempt = Attempt::fresh();
            let login = OAuthBrowserLogin::with_callback_timeout_secs(
                OAuthEndpoints::new(Some(attempt.base())),
                1,
            );
            let started = std::time::Instant::now();
            (login.receive(LoginProvider::Xai, &attempt.state()), started.elapsed())
        })
        .find(|(received, _)| {
            !matches!(received, Err(AuthorizationCallbackError::Failed(why)) if why.contains("failed to bind"))
        })
        .expect("three ports taken before the listener bound them");
    assert_eq!(
        received.0,
        Err(AuthorizationCallbackError::Failed(
            "provider error: OAuth callback timed out".to_string()
        ))
    );
    assert!(received.1 < Duration::from_secs(10));
}
