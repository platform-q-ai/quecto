//! Contract for the auth capability's `AuthorizationCallback` (#1839): the
//! loopback listener on the provider's redirect URI returns the code of
//! the callback carrying the login's state, keeps waiting past one that
//! does not, reports an OAuth error callback as a failure the user may
//! paste past, gives up at its deadline, and refuses a redirect URI it must
//! not listen on. Only the
//! test itself calls the listener; no provider is contacted.
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

/// Request `path` from the listener on `port` once it accepts, and return
/// its answer.
fn call(port: u16, path: &str) -> String {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut stream = loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) => break stream,
            Err(error) if std::time::Instant::now() < deadline => {
                let _ = error;
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => panic!("the listener never accepted: {error}"),
        }
    };
    stream
        .write_all(format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes())
        .unwrap();
    let mut answer = String::new();
    let _ = stream.read_to_string(&mut answer);
    answer
}

/// How long a listener under test waits: long enough for a loaded
/// runner, short enough that a lost port race costs seconds, not the
/// production 300.
const TEST_CALLBACK_SECS: u64 = 15;

/// The xAI listener on a test server's loopback redirect, receiving on a
/// thread while `calls` run against it. `calls` answers whether every
/// answer came from this listener: a free port can be taken by another
/// test's server between its release and the listener's bind, and then
/// the attempt is repeated on a fresh port.
fn receive_while(
    calls: impl Fn(u16) -> bool + Send + Sync + Clone + 'static,
) -> Result<String, AuthorizationCallbackError> {
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
        if caller.join().unwrap() {
            return received;
        }
    }
    panic!("three free ports were taken before the listener bound them");
}

/// Whether `answer` is the listener's (a port race answers otherwise).
fn from_the_listener(answer: &str, status: &str) -> bool {
    answer.starts_with(status)
}

#[test]
fn the_callback_carrying_the_state_returns_its_code() {
    let received = receive_while(|port| {
        let answer = call(port, "/callback?code=fake-code&state=fake-state");
        from_the_listener(&answer, "HTTP/1.1 200")
    });
    assert_eq!(received, Ok("fake-code".to_string()));
}

#[test]
fn a_callback_with_another_state_is_refused_and_waiting_goes_on() {
    let received = receive_while(|port| {
        let refused = call(port, "/callback?code=forged&state=other");
        if !from_the_listener(
            &refused,
            "HTTP/1.1 400 Bad Request\r\nConnection: close\r\nContent-Length: 14\r\n\r\nState mismatch",
        ) {
            return false;
        }
        from_the_listener(
            &call(port, "/callback?code=fake-code&state=fake-state"),
            "HTTP/1.1 200",
        )
    });
    assert_eq!(received, Ok("fake-code".to_string()));
}

#[test]
fn an_oauth_error_callback_is_a_failure_to_paste_past() {
    let received = receive_while(|port| {
        from_the_listener(
            &call(port, "/callback?error=access_denied&state=fake-state"),
            "HTTP/1.1 400 Bad Request\r\nConnection: close\r\nContent-Length: 12\r\n\r\nAuth failure",
        )
    });
    assert!(
        matches!(received, Err(AuthorizationCallbackError::Failed(_))),
        "{received:?}"
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
