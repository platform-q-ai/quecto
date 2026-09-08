//! P2 AC3/4: existing retry owns attempts and sleeps outside transport permits.
// This target reuses the loopback fixture subset; other surfaces run separately.
#[path = "common/admission_attempt_fixture.rs"]
pub mod fixture;
use fixture::{Gate, Leaf, Surface, bounded, invoke};
use quecto::infrastructure::providers::retry::{RetryConfig, RetryingProvider};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn retry_exhaustion_charges_each_send_and_sleeps_after_finish() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(529).set_body_string("busy"))
        .expect(3)
        .mount(&server)
        .await;
    let gate = Gate::new(false);
    let sleeps = Arc::new(AtomicUsize::new(0));
    let observed_gate = gate.clone();
    let observed_sleeps = sleeps.clone();
    let retry = RetryingProvider::with_sleeper(
        Leaf::OpenAi.provider(&server.uri(), gate.clone()),
        RetryConfig::no_delay(3),
        Arc::new(move |_| {
            let gate = observed_gate.clone();
            let sleeps = observed_sleeps.clone();
            Box::pin(async move {
                assert_eq!(
                    gate.active(),
                    0,
                    "retry sleep must not retain transport occupancy"
                );
                sleeps.fetch_add(1, Ordering::SeqCst);
            })
        }),
    );
    let result = bounded(invoke(&retry, Surface::Chat)).await;
    assert!(result.is_err());
    assert_eq!(sleeps.load(Ordering::SeqCst), 2);
    assert_eq!(server.received_requests().await.unwrap().len(), 3);
    assert_eq!(
        gate.events()
            .iter()
            .filter(|e| matches!(e, fixture::Event::Granted(_)))
            .count(),
        3
    );
}

#[tokio::test]
async fn auth_refresh_rebuild_reacquires_same_bound_gate() {
    use quecto::infrastructure::auth::credential_store::{AuthMethod, Credential, CredentialStore};
    use quecto::infrastructure::providers::openai::OpenAiProvider;
    use quecto::infrastructure::providers::refreshable::{RefreshableConfig, RefreshableProvider};
    use wiremock::matchers::header;
    let server = MockServer::start().await;
    Mock::given(header("authorization", "Bearer old"))
        .respond_with(ResponseTemplate::new(401).set_body_string("expired"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(header("authorization", "Bearer fresh"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({"choices":[{"message":{"content":"fresh"},"finish_reason":"stop"}]}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let gate = Gate::new(false);
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(CredentialStore::new(dir.path()));
    store
        .store(Credential {
            provider: "openai".into(),
            token: "old".into(),
            method: AuthMethod::OAuth,
            expires_at: Some(0),
            refresh_token: Some("refresh".into()),
            account_id: None,
        })
        .unwrap();
    let bound_gate = gate.clone();
    let url = server.uri();
    let refreshed_gate = gate.clone();
    let provider = RefreshableProvider::new(RefreshableConfig {
        inner: Arc::new(
            OpenAiProvider::with_client(
                "old".into(),
                Some(url.clone()),
                reqwest::Client::builder().no_proxy().build().unwrap(),
            )
            .with_attempt_admission(
                gate.clone(),
                quecto::infrastructure::providers::SingleAttemptClient::build(
                    reqwest::Client::builder().no_proxy(),
                )
                .unwrap(),
            ),
        ),
        store,
        provider_name: "openai".into(),
        credential_provider: "openai".into(),
        refresh_fn: Arc::new(move |_, _| {
            let gate = refreshed_gate.clone();
            Box::pin(async move {
                assert_eq!(
                    gate.active(),
                    0,
                    "refresh occurs after failed transport finished"
                );
                Ok("fresh".into())
            })
        }),
        factory: Arc::new(move |token| {
            Arc::new(
                OpenAiProvider::with_client(
                    token.to_string(),
                    Some(url.clone()),
                    reqwest::Client::builder().no_proxy().build().unwrap(),
                )
                .with_attempt_admission(
                    bound_gate.clone(),
                    quecto::infrastructure::providers::SingleAttemptClient::build(
                        reqwest::Client::builder().no_proxy(),
                    )
                    .unwrap(),
                ),
            )
        }),
    });
    assert_eq!(
        bounded(invoke(&provider, Surface::Chat)).await.unwrap(),
        "fresh"
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
    assert_eq!(
        gate.events()
            .iter()
            .filter(|e| matches!(e, fixture::Event::Granted(_)))
            .count(),
        2
    );
}
