use super::*;

fn no_threads() -> std::io::Result<tokio::runtime::Runtime> {
    Err(std::io::Error::from_raw_os_error(libc::EAGAIN))
}

fn unavailable_login() -> OAuthBrowserLogin {
    OAuthBrowserLogin::on_runtimes(
        OAuthEndpoints::new(Some("http://127.0.0.1:1".to_string())),
        no_threads,
    )
}

#[test]
fn a_process_that_cannot_start_a_thread_cannot_wait_or_exchange() {
    let login = unavailable_login();
    for provider in [LoginProvider::OpenAi, LoginProvider::Xai] {
        assert!(
            matches!(
                login.receive(provider, "fake-state"),
                Err(AuthorizationCallbackError::Unavailable(ref reason))
                    if reason.contains("temporarily unavailable")
            ),
            "{provider:?}"
        );
    }
    for provider in LoginProvider::ALL {
        assert!(
            matches!(
                login.exchange_code(provider, "fake-code", "fake-verifier"),
                Err(TokenExchangeError::Unavailable(ref reason))
                    if reason.contains("temporarily unavailable")
            ),
            "{provider:?}"
        );
    }
}

#[tokio::test]
async fn a_call_from_inside_a_runtime_is_unavailable_not_a_panic() {
    let login = OAuthBrowserLogin::new(OAuthEndpoints::new(Some("http://127.0.0.1:1".to_string())));
    let inside = "called from inside an async runtime".to_string();
    assert_eq!(
        login.receive(LoginProvider::Xai, "fake-state"),
        Err(AuthorizationCallbackError::Unavailable(inside.clone()))
    );
    assert_eq!(
        login.exchange_code(LoginProvider::OpenAi, "fake-code", "fake-verifier"),
        Err(TokenExchangeError::Unavailable(inside))
    );
}

#[test]
fn starting_an_authorization_needs_no_runtime() {
    // Building the URL is local: no thread, no network.
    let pending = unavailable_login().begin(LoginProvider::OpenAi).unwrap();
    assert!(
        pending
            .authorization_url
            .starts_with("http://127.0.0.1:1/authorize?")
    );
}
