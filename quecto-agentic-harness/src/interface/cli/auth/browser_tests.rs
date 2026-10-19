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
    // A short wait: a lost port race (see the test) costs seconds.
    let listener = Arc::new(OAuthBrowserLogin::with_callback_timeout_secs(
        OAuthEndpoints::new(environment.oauth_base_url.clone()),
        15,
    ));
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

/// The listener's success page, head and body marker as it writes them.
const SUCCESS_HEAD: &str = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n";
const SUCCESS_MARK: &str = "Authentication successful!";

fn succeeded(answer: &Option<String>) -> bool {
    answer
        .as_deref()
        .is_some_and(|answer| answer.starts_with(SUCCESS_HEAD) && answer.contains(SUCCESS_MARK))
}

/// Request `path` from whatever accepts on `port`, within bounds: up to
/// 10 s to connect, 10 s to read, the connection closed after one answer.
/// `None` when nothing answered in time.
fn call(port: u16, path: &str) -> Option<String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut stream = loop {
        match std::net::TcpStream::connect(("127.0.0.1", port)) {
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

fn login_args() -> Vec<String> {
    ["login", "--provider", "xai"].map(String::from).to_vec()
}

#[test]
fn a_callback_carrying_the_state_stores_the_credential_without_a_paste_prompt() {
    // A free port can be taken by another test's server between its
    // release and the listener's bind. Only a run whose listener could not
    // bind, or timed out without the call reaching it, is repeated.
    let mut last = None;
    let mut outcome = None;
    for _attempt in 0..3 {
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
        let answer = caller.join().unwrap();
        let lost_a_race = !succeeded(&answer)
            && (stdout.contains("failed to bind") || stdout.contains("OAuth callback timed out"));
        if !lost_a_race {
            outcome = Some((base, code, stdout, stderr, answer));
            break;
        }
        last = Some((stdout, stderr, answer));
    }
    let Some((base, code, stdout, stderr, answer)) = outcome else {
        panic!("three port races in a row; last attempt: {last:?}");
    };
    assert!(succeeded(&answer), "{answer:?} / {stdout} / {stderr}");
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
