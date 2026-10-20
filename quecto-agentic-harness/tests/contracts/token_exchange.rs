//! Contract for the auth capability's `TokenExchange` (#1839, #1840): a
//! code and its PKCE verifier, or a refresh token, are exchanged in each
//! provider's wire shape; OpenAI's token names its account, no other
//! provider's does; a refusal names the status, never the body. Every
//! request goes to a local mock.
use base64::Engine;
use wiremock::matchers::{body_string_contains, method, path};

use quecto::application::auth::ports::{TokenExchange, TokenExchangeError};
use quecto::domain::auth::provider::LoginProvider;
use quecto::infrastructure::auth::browser_login::OAuthBrowserLogin;
use quecto::infrastructure::auth::oauth::OAuthEndpoints;

/// A mock token endpoint answering requests whose body contains every one
/// of `expects`, kept alive on its own runtime.
struct TokenServer {
    _runtime: tokio::runtime::Runtime,
    server: wiremock::MockServer,
}

impl TokenServer {
    fn answering(expects: &[&str], status: u16, body: serde_json::Value) -> Self {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let server = runtime.block_on(async {
            let server = wiremock::MockServer::start().await;
            let mut mock = wiremock::Mock::given(method("POST")).and(path("/oauth/token"));
            for expected in expects {
                mock = mock.and(body_string_contains(*expected));
            }
            mock.respond_with(wiremock::ResponseTemplate::new(status).set_body_json(&body))
                .mount(&server)
                .await;
            server
        });
        Self {
            _runtime: runtime,
            server,
        }
    }

    fn exchange(&self) -> OAuthBrowserLogin {
        OAuthBrowserLogin::new(OAuthEndpoints::new(Some(self.server.uri())))
    }
}

fn jwt_naming(account: &str) -> String {
    let claims = serde_json::json!({
        "https://api.openai.com/auth": { "chatgpt_account_id": account }
    });
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(&claims).unwrap());
    format!("header.{payload}.signature")
}

fn issued(access: &str, refresh: Option<&str>) -> serde_json::Value {
    let mut body = serde_json::json!({ "access_token": access, "expires_in": 3600 });
    if let Some(refresh) = refresh {
        body["refresh_token"] = refresh.into();
    }
    body
}

#[test]
fn an_openai_exchange_is_form_encoded_and_names_the_account() {
    let jwt = jwt_naming("acct-fake");
    let server = TokenServer::answering(
        &[
            "grant_type=authorization_code",
            "code=fake-code",
            "code_verifier=fake-verifier",
        ],
        200,
        issued(&jwt, Some("fake-refresh")),
    );
    let token = server
        .exchange()
        .exchange_code(LoginProvider::OpenAi, "fake-code", "fake-verifier")
        .unwrap();
    assert_eq!(token.access_token, jwt);
    assert_eq!(token.refresh_token.as_deref(), Some("fake-refresh"));
    assert_eq!(token.expires_in, 3600);
    assert_eq!(token.account_id.as_deref(), Some("acct-fake"));
    assert!(!format!("{token:?}").contains("fake-refresh"));
}

#[test]
fn an_openai_token_that_is_no_jwt_names_no_account() {
    let server = TokenServer::answering(&[], 200, issued("plain-token", None));
    let token = server
        .exchange()
        .exchange_code(LoginProvider::OpenAi, "fake-code", "fake-verifier")
        .unwrap();
    assert_eq!(token.account_id, None);
    assert_eq!(token.refresh_token, None);
}

#[test]
fn an_anthropic_exchange_splits_the_pasted_code_from_its_state() {
    let server = TokenServer::answering(
        &[
            r#""code":"fake-code""#,
            r#""state":"fake-state""#,
            r#""code_verifier":"fake-verifier""#,
        ],
        200,
        issued("fake-access", Some("fake-refresh")),
    );
    let token = server
        .exchange()
        .exchange_code(
            LoginProvider::Anthropic,
            "fake-code#fake-state",
            "fake-verifier",
        )
        .unwrap();
    assert_eq!(token.access_token, "fake-access");
    assert_eq!(token.account_id, None);
}

#[test]
fn only_openai_tokens_name_an_account() {
    let server = TokenServer::answering(
        &["code=fake-code"],
        200,
        issued(&jwt_naming("acct-fake"), None),
    );
    let token = server
        .exchange()
        .exchange_code(LoginProvider::Xai, "fake-code", "fake-verifier")
        .unwrap();
    assert_eq!(token.account_id, None);
}

#[test]
fn a_refusal_names_the_status_and_never_the_body() {
    for provider in LoginProvider::ALL {
        let server = TokenServer::answering(&[], 400, serde_json::json!("fake-leaky-internals"));
        match server
            .exchange()
            .exchange_code(provider, "fake-code", "fake-verifier")
        {
            Err(TokenExchangeError::Rejected(reason)) => {
                assert!(reason.contains("400"), "{provider:?}: {reason}");
                assert!(!reason.contains("fake-leaky-internals"), "{reason}");
            }
            other => panic!("{provider:?}: expected a refusal, got {other:?}"),
        }
    }
}

#[test]
fn a_refresh_uses_each_providers_wire_shape() {
    let jwt = jwt_naming("acct-fake");
    for (provider, expects) in [
        (
            LoginProvider::OpenAi,
            vec!["grant_type=refresh_token", "refresh_token=fake-refresh"],
        ),
        (
            LoginProvider::Xai,
            vec!["grant_type=refresh_token", "refresh_token=fake-refresh"],
        ),
        (
            LoginProvider::Anthropic,
            vec![
                r#""grant_type":"refresh_token""#,
                r#""refresh_token":"fake-refresh""#,
            ],
        ),
    ] {
        let server = TokenServer::answering(&expects, 200, issued(&jwt, None));
        let token = server
            .exchange()
            .refresh(provider, "fake-refresh")
            .unwrap_or_else(|error| panic!("{provider:?}: {error:?}"));
        assert_eq!(token.access_token, jwt);
        assert_eq!(token.refresh_token, None, "absent means keep the old one");
        assert_eq!(
            token.account_id.is_some(),
            provider == LoginProvider::OpenAi,
            "{provider:?}"
        );
    }
}

#[test]
fn a_refused_refresh_names_the_status_and_never_the_body() {
    for provider in LoginProvider::ALL {
        let server = TokenServer::answering(&[], 401, serde_json::json!("fake-leaky-internals"));
        match server.exchange().refresh(provider, "fake-refresh") {
            Err(TokenExchangeError::Rejected(reason)) => {
                assert!(reason.contains("401"), "{provider:?}: {reason}");
                assert!(!reason.contains("fake-leaky-internals"), "{reason}");
            }
            other => panic!("{provider:?}: expected a refusal, got {other:?}"),
        }
    }
}
