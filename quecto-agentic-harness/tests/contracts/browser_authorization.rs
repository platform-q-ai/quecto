//! Contract for the auth capability's `BrowserAuthorization` (#1839): each
//! login provider's authorization URL carries a fresh state and the S256
//! challenge of a verifier that never appears in it, and says how the code
//! comes back. No request is made.
use base64::Engine;
use sha2::Digest;

use quecto::application::auth::ports::{
    AuthorizationReturn, BrowserAuthorization, PendingAuthorization,
};
use quecto::domain::auth::provider::LoginProvider;
use quecto::infrastructure::auth::browser_login::OAuthBrowserLogin;
use quecto::infrastructure::auth::oauth::OAuthEndpoints;

fn begin(endpoints: Option<&str>, provider: LoginProvider) -> PendingAuthorization {
    OAuthBrowserLogin::new(OAuthEndpoints::new(endpoints.map(str::to_string)))
        .begin(provider)
        .expect("every login provider has an OAuth configuration")
}

fn param(url: &str, name: &str) -> String {
    let query = url.split_once('?').expect("a query").1;
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix(&format!("{name}=")))
        .map(|value| urlencoding::decode(value).unwrap().into_owned())
        .unwrap_or_else(|| panic!("{name} missing from {url}"))
}

#[test]
fn each_provider_authorizes_at_its_own_endpoint_and_returns_its_own_way() {
    let cases = [
        (
            LoginProvider::OpenAi,
            "https://auth.openai.com/oauth/authorize?",
            AuthorizationReturn::Loopback {
                waiting_on: "http://localhost:1455".to_string(),
            },
        ),
        (
            LoginProvider::Xai,
            "https://auth.x.ai/oauth2/authorize?",
            AuthorizationReturn::Loopback {
                waiting_on: "http://127.0.0.1:56121/callback".to_string(),
            },
        ),
        (
            LoginProvider::Anthropic,
            "https://claude.ai/oauth/authorize?",
            AuthorizationReturn::PastedCode,
        ),
    ];
    for (provider, prefix, returns) in cases {
        let pending = begin(None, provider);
        assert!(
            pending.authorization_url.starts_with(prefix),
            "{provider:?}: {}",
            pending.authorization_url
        );
        assert_eq!(pending.returns, returns, "{provider:?}");
    }
}

#[test]
fn the_url_binds_the_state_and_the_verifiers_challenge_only() {
    for provider in LoginProvider::ALL {
        let pending = begin(None, provider);
        let url = &pending.authorization_url;
        assert_eq!(pending.state.len(), 32, "{provider:?}");
        assert_eq!(param(url, "state"), pending.state);
        assert_eq!(param(url, "code_challenge_method"), "S256");
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(sha2::Sha256::digest(pending.verifier.as_bytes()));
        assert_eq!(param(url, "code_challenge"), challenge, "{provider:?}");
        assert!(
            !url.contains(&pending.verifier),
            "the verifier stays secret"
        );
        assert!(!format!("{pending:?}").contains(&pending.verifier));
    }
}

#[test]
fn every_authorization_is_fresh() {
    let first = begin(None, LoginProvider::OpenAi);
    let second = begin(None, LoginProvider::OpenAi);
    assert_ne!(first.state, second.state);
    assert_ne!(first.verifier, second.verifier);
}

#[test]
fn a_test_server_stands_in_for_every_provider() {
    for provider in LoginProvider::ALL {
        let pending = begin(Some("http://127.0.0.1:9"), provider);
        assert!(
            pending
                .authorization_url
                .starts_with("http://127.0.0.1:9/authorize?"),
            "{provider:?}: {}",
            pending.authorization_url
        );
    }
}
