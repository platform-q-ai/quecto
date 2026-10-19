//! The browser callback arms of `auth login` (#2591 review): with no
//! supplied input the CLI waits on the real loopback listener, and a
//! callback that arrives (or a redirect it cannot listen on) never brings
//! up the paste prompt.
use std::io::{Read, Write};
use std::sync::Arc;
use std::time::Duration;

use crate::application::auth::ports::{
    AuthorizationReturn, BrowserAuthorization, BrowserAuthorizationError, IssuedToken,
    PendingAuthorization, TokenExchange, TokenExchangeError,
};
use crate::application::auth::use_cases::LogInWithOAuth;
use crate::domain::auth::credential::AuthMethod;
use crate::domain::auth::provider::LoginProvider;
use crate::infrastructure::auth::browser_login::OAuthBrowserLogin;
use crate::infrastructure::auth::credential_store::CredentialStore;
use crate::infrastructure::auth::oauth::OAuthEndpoints;
use crate::infrastructure::time::auth_clock::SystemAuthClock;
use crate::interface::cli::CliContext;
use crate::interface::cli::auth::{AuthEnvironment, AuthHandles, cmd_auth_with_reader};

/// An authorization bound to a known state, returning to the run's
/// loopback redirect.
struct KnownState;

impl BrowserAuthorization for KnownState {
    fn begin(
        &self,
        _provider: LoginProvider,
    ) -> Result<PendingAuthorization, BrowserAuthorizationError> {
        Ok(PendingAuthorization {
            authorization_url: "https://auth.example.test/authorize".to_string(),
            state: "fake-state".to_string(),
            verifier: "fake-verifier".to_string(),
            returns: AuthorizationReturn::Loopback {
                waiting_on: "the test listener".to_string(),
            },
        })
    }
}

/// A token endpoint that issues for the callback's code only.
struct IssuesForTheCallbackCode;

impl TokenExchange for IssuesForTheCallbackCode {
    fn exchange_code(
        &self,
        _provider: LoginProvider,
        code: &str,
        _verifier: &str,
    ) -> Result<IssuedToken, TokenExchangeError> {
        match code {
            "fake" => Ok(IssuedToken {
                access_token: "fake-access".to_string(),
                refresh_token: None,
                expires_in: 3600,
                account_id: None,
            }),
            other => Err(TokenExchangeError::Rejected(format!(
                "unexpected code {other}"
            ))),
        }
    }
}

/// Composition's handles with the browser login over the real loopback
/// listener on the run's (test) redirect, a known state and a local token
/// endpoint.
fn callback_rig(environment: &AuthEnvironment) -> AuthHandles {
    let mut handles = crate::composition::bootstrap::auth::build_auth_handles(environment);
    let listener = Arc::new(OAuthBrowserLogin::new(OAuthEndpoints::new(
        environment.oauth_base_url.clone(),
    )));
    handles.oauth_login = Arc::new(LogInWithOAuth::new(
        Arc::new(KnownState),
        listener,
        Arc::new(IssuesForTheCallbackCode),
        Arc::new(CredentialStore::new(&environment.base_dir)),
        Arc::new(SystemAuthClock),
    ));
    handles
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Request `path` from the listener on `port` once it accepts.
fn call(port: u16, path: &str) -> String {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut stream = loop {
        match std::net::TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) => break stream,
            Err(_) if std::time::Instant::now() < deadline => {
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

fn login_args() -> Vec<String> {
    ["login", "--provider", "xai"].map(String::from).to_vec()
}

#[test]
fn a_callback_carrying_the_state_stores_the_credential_without_a_paste_prompt() {
    let base = tempfile::TempDir::new().unwrap();
    let port = free_port();
    let ctx = CliContext {
        base_dir: Some(base.path().to_path_buf()),
        oauth_base_url: Some(format!("http://127.0.0.1:{port}")),
        auth: Some(callback_rig),
        ..Default::default()
    };
    let caller = std::thread::spawn(move || call(port, "/callback?code=fake&state=fake-state"));
    let (mut stdout, mut stderr) = (String::new(), String::new());
    let code = cmd_auth_with_reader(
        &ctx,
        &login_args(),
        &mut stdout,
        &mut stderr,
        &mut std::io::empty(),
    );
    assert!(caller.join().unwrap().starts_with("HTTP/1.1 200"));
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert!(!stdout.contains("Paste"), "{stdout}");
    assert!(
        stdout.ends_with("Xai OAuth credential stored successfully\n"),
        "{stdout}"
    );
    let stored = CredentialStore::new(base.path())
        .get("xai")
        .unwrap()
        .unwrap();
    assert_eq!(stored.token, "fake-access");
    assert_eq!(stored.method, AuthMethod::OAuth);
}

#[test]
fn a_redirect_off_this_machine_is_refused_without_a_paste_prompt() {
    let base = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(base.path().to_path_buf()),
        oauth_base_url: Some("http://example.test".to_string()),
        auth: Some(crate::composition::bootstrap::auth::build_auth_handles),
        ..Default::default()
    };
    let (mut stdout, mut stderr) = (String::new(), String::new());
    let code = cmd_auth_with_reader(
        &ctx,
        &login_args(),
        &mut stdout,
        &mut stderr,
        &mut std::io::empty(),
    );
    assert_eq!(code, 1);
    assert_eq!(
        stderr,
        "auth login: invalid redirect URI: provider error: redirect URI host is not loopback: \
         http://example.test/callback\n"
    );
    assert!(!stdout.contains("Paste"), "{stdout}");
    assert!(!base.path().join("credentials.json").exists());
}
