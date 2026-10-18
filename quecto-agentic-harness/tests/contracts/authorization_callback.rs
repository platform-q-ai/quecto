//! Contract for the auth capability's `AuthorizationCallback` (#1839): the
//! loopback listener on the provider's redirect URI returns the code of
//! the callback carrying the login's state, keeps waiting past one that
//! does not, reports an OAuth error callback as a failure the user may
//! paste past, and refuses a redirect URI it must not listen on. Only the
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

/// The xAI listener on a test server's loopback redirect, receiving on a
/// thread while `calls` run against it.
fn receive_while(
    calls: impl FnOnce(u16) + Send + 'static,
) -> Result<String, AuthorizationCallbackError> {
    let port = free_port();
    let login = OAuthBrowserLogin::new(OAuthEndpoints::new(Some(format!(
        "http://127.0.0.1:{port}"
    ))));
    let caller = std::thread::spawn(move || calls(port));
    let received = login.receive(LoginProvider::Xai, "fake-state");
    caller.join().unwrap();
    received
}

#[test]
fn the_callback_carrying_the_state_returns_its_code() {
    let received = receive_while(|port| {
        let answer = call(port, "/callback?code=fake-code&state=fake-state");
        assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    });
    assert_eq!(received, Ok("fake-code".to_string()));
}

#[test]
fn a_callback_with_another_state_is_refused_and_waiting_goes_on() {
    let received = receive_while(|port| {
        let refused = call(port, "/callback?code=forged&state=other");
        assert!(refused.starts_with("HTTP/1.1 400"), "{refused}");
        call(port, "/callback?code=fake-code&state=fake-state");
    });
    assert_eq!(received, Ok("fake-code".to_string()));
}

#[test]
fn an_oauth_error_callback_is_a_failure_to_paste_past() {
    let received = receive_while(|port| {
        call(port, "/callback?error=access_denied&state=fake-state");
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
