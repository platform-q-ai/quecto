//! Contract for the auth capability's `DeviceCodeFlow` (#1839) over the
//! providers' OAuth endpoints: a grant names only where to go and what to
//! enter; a refusal names the status, never the response body; a provider
//! without a device-code endpoint is unsupported without any request. Every
//! request goes to a local mock: no real provider is ever contacted.
use std::sync::Arc;

use quecto::application::auth::ports::{DeviceCodeError, DeviceCodeFlow, DeviceCodeGrant};
use quecto::domain::auth::provider::LoginProvider;
use quecto::infrastructure::auth::device_code_flow::OAuthDeviceCodeFlow;
use quecto::infrastructure::auth::oauth::OAuthEndpoints;

/// A mock OAuth server answering `POST /device/code`, kept alive on its
/// own multi-thread runtime while the adapter runs its own.
struct MockServer {
    _runtime: tokio::runtime::Runtime,
    server: wiremock::MockServer,
}

impl MockServer {
    fn answering(status: u16, body: serde_json::Value) -> Self {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let server = runtime.block_on(async {
            let server = wiremock::MockServer::start().await;
            wiremock::Mock::given(wiremock::matchers::method("POST"))
                .and(wiremock::matchers::path("/device/code"))
                .respond_with(wiremock::ResponseTemplate::new(status).set_body_json(&body))
                .mount(&server)
                .await;
            server
        });
        Self {
            _runtime: runtime,
            server,
        }
    }

    fn flow(&self) -> Arc<dyn DeviceCodeFlow> {
        Arc::new(OAuthDeviceCodeFlow::new(OAuthEndpoints::new(Some(
            self.server.uri(),
        ))))
    }
}

#[test]
fn a_grant_names_where_to_go_and_what_to_enter() {
    let mock = MockServer::answering(
        200,
        serde_json::json!({
            "device_code": "fake-device-secret",
            "user_code": "ABCD-1234",
            "verification_uri": "https://example.test/device",
        }),
    );
    assert_eq!(
        mock.flow().request(LoginProvider::OpenAi),
        Ok(DeviceCodeGrant {
            verification_uri: "https://example.test/device".to_string(),
            user_code: "ABCD-1234".to_string(),
        })
    );
}

#[test]
fn a_refusal_names_the_status_and_never_the_body() {
    let mock = MockServer::answering(500, serde_json::json!("fake-leaky-internals"));
    match mock.flow().request(LoginProvider::OpenAi) {
        Err(DeviceCodeError::Request(reason)) => {
            assert!(reason.contains("500"), "{reason}");
            assert!(!reason.contains("fake-leaky-internals"), "{reason}");
        }
        other => panic!("expected a refused request, got {other:?}"),
    }
}

#[test]
fn an_unparseable_grant_is_a_failed_request() {
    let mock = MockServer::answering(200, serde_json::json!({ "user_code": "only" }));
    assert!(matches!(
        mock.flow().request(LoginProvider::OpenAi),
        Err(DeviceCodeError::Request(_))
    ));
}

#[test]
fn providers_without_a_device_code_endpoint_are_unsupported_offline() {
    let flow = OAuthDeviceCodeFlow::new(OAuthEndpoints::new(None));
    for provider in [LoginProvider::Xai, LoginProvider::Anthropic] {
        assert_eq!(
            flow.request(provider),
            Err(DeviceCodeError::Unsupported),
            "{provider:?}"
        );
    }
}
