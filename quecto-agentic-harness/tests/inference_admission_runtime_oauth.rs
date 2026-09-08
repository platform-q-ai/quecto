//! Real runtime OAuth refresh: stable admission binding, rotated wire credentials,
//! and the unchanged disabled callback contract. No test-local composition.
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use quecto::application::ports::{AttemptAdmission, AttemptPermit, ProviderRuntimeFactory};
use quecto::domain::error::DomainError;
use quecto::domain::inference_admission::*;
use quecto::domain::provider::{ChatRequest, LlmProvider};
use quecto::infrastructure::auth::credential_store::{AuthMethod, Credential, CredentialStore};
use quecto::infrastructure::config::Config;
use quecto::infrastructure::model_registry::ModelRegistry;
use quecto::infrastructure::provider_runtime::{AgentProviderRuntimeFactory, AgentRuntimeInputs};
use quecto::infrastructure::provider_runtime_admission::*;
use quecto::infrastructure::providers::codex::CodexProvider;
use tokio::sync::{Notify, Semaphore};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const LIMIT: Duration = Duration::from_secs(5);

#[derive(Debug)]
struct Gate {
    starts: AtomicUsize,
    finishes: Arc<Mutex<Vec<Feedback>>>,
    second_waiting: Notify,
    allow_second: Semaphore,
}
impl Gate {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            starts: AtomicUsize::new(0),
            finishes: Arc::new(Mutex::new(vec![])),
            second_waiting: Notify::new(),
            allow_second: Semaphore::new(0),
        })
    }
}
#[derive(Debug)]
struct Permit(Arc<Mutex<Vec<Feedback>>>);
impl AttemptPermit for Permit {
    fn feedback(&mut self, _: ThrottleFeedback) {
        panic!("401 is not throttle feedback")
    }
    fn finish(self: Box<Self>, feedback: Feedback) {
        self.0.lock().unwrap().push(feedback);
    }
}
impl AttemptAdmission for Gate {
    fn acquire(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn AttemptPermit>, DomainError>> + Send + '_>>
    {
        Box::pin(async move {
            let attempt = self.starts.fetch_add(1, Ordering::SeqCst);
            if attempt == 1 {
                self.second_waiting.notify_one();
                self.allow_second.acquire().await.unwrap().forget();
            }
            Ok(Box::new(Permit(self.finishes.clone())) as Box<dyn AttemptPermit>)
        })
    }
}
fn token(account: &str) -> String {
    let payload = serde_json::json!({"https://api.openai.com/auth":{"chatgpt_account_id":account}});
    format!(
        "header.{}.signature",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload.to_string())
    )
}
fn credential(token: String) -> Credential {
    Credential {
        provider: "openai".into(),
        token,
        method: AuthMethod::OAuth,
        expires_at: None,
        refresh_token: Some("refresh-fixture".into()),
        account_id: None,
    }
}
fn request() -> ChatRequest<'static> {
    static MESSAGES: std::sync::LazyLock<Vec<quecto::domain::message::Message>> =
        std::sync::LazyLock::new(|| {
            vec![quecto::domain::message::Message::system(
                "Runtime OAuth fixture",
            )]
        });
    ChatRequest {
        messages: &MESSAGES,
        tools: &[],
        model: "openai-oauth/retained-model",
        max_tokens: 32,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    }
}

async fn actual_runtime_oauth_refresh(enabled: bool) {
    let server = MockServer::start().await;
    let old = token("old-account");
    let rotated = token("rotated-account");
    Mock::given(method("POST"))
        .and(path("/codex/responses"))
        .and(header("authorization", format!("Bearer {old}")))
        .respond_with(ResponseTemplate::new(401).set_body_json(
            serde_json::json!({"error":{"message":"expired","type":"authentication_error"}}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST")).and(path("/codex/responses")).and(header("authorization", format!("Bearer {rotated}")))
        .respond_with(ResponseTemplate::new(200).insert_header("content-type", "text/event-stream").set_body_string("data: {\"type\":\"response.output_text.delta\",\"delta\":\"runtime-oauth-ok\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n\n"))
        .mount(&server).await;
    let dir = tempfile::tempdir().unwrap();
    CredentialStore::new(dir.path())
        .store(credential(old.clone()))
        .unwrap();
    let refreshed = Arc::new(AtomicUsize::new(0));
    let callbacks = Arc::new(AtomicUsize::new(0));
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let inputs = AgentRuntimeInputs {
        base_dir: dir.path().into(),
        http_client: client.clone(),
        refresh_fn: {
            let refreshed = refreshed.clone();
            let rotated = rotated.clone();
            Arc::new(move |store, provider| {
                assert_eq!(provider, "openai");
                refreshed.fetch_add(1, Ordering::SeqCst);
                let rotated = rotated.clone();
                Box::pin(async move {
                    store.store(credential(rotated.clone()))?;
                    Ok(rotated)
                })
            })
        },
        openai_oauth_factory: {
            let callbacks = callbacks.clone();
            let base = server.uri();
            Arc::new(move |token| {
                callbacks.fetch_add(1, Ordering::SeqCst);
                Arc::new(CodexProvider::with_client(
                    token.into(),
                    "rotated-account".into(),
                    Some(base.clone()),
                    client.clone(),
                ))
            })
        },
        model_registry: Ok(ModelRegistry::from_file_records(vec![])),
    };
    let mut config = Config::default();
    config.providers.openai.api_base = server.uri();
    let gate = Gate::new();
    let provider: Arc<dyn LlmProvider> = if enabled {
        let group = GroupId::new("quota").unwrap();
        let proposal = AdmissionRuntimeProposal {
            policy: AdmissionConfig {
                groups: BTreeMap::from([(
                    group.clone(),
                    GroupPolicy {
                        capacity: 1,
                        reserve: 0,
                        min_interval_ms: 1,
                        queue_capacity: 2,
                        queue_timeout_ms: 100,
                        attempt_timeout_ms: 100,
                        fallback_base_ms: 100,
                        max_cooldown_ms: 100,
                    },
                )]),
                aliases: BTreeMap::from([("stable-explicit-account".into(), group)]),
                max_scopes: 2,
                terminal_capacity: 2,
            },
            bindings: BTreeMap::from([("openai-oauth".into(), "stable-explicit-account".into())]),
        };
        let context = AdmissionRuntimeContext::new(
            proposal.clone(),
            BTreeMap::from([(
                "stable-explicit-account".into(),
                gate.clone() as Arc<dyn AttemptAdmission>,
            )]),
            quecto::infrastructure::providers::SingleAttemptClient::build(
                reqwest::Client::builder().no_proxy(),
            )
            .unwrap(),
        )
        .unwrap();
        AdmissionProviderRuntimeFactory::new(Arc::new(context))
            .compose_runtime(
                &AdmissionRuntimeCandidate {
                    providers: &config,
                    admission: Some(&proposal),
                },
                &inputs,
            )
            .unwrap()
    } else {
        AgentProviderRuntimeFactory
            .compose_runtime(&config, &inputs)
            .unwrap()
    };
    let task = tokio::spawn(async move { provider.chat(request()).await });
    if enabled {
        tokio::time::timeout(LIMIT, gate.second_waiting.notified())
            .await
            .expect("rebuilt leaf must acquire the original bound gate");
        assert_eq!(
            gate.starts.load(Ordering::SeqCst),
            2,
            "one fresh grant per physical attempt"
        );
        assert_eq!(
            *gate.finishes.lock().unwrap(),
            [Feedback::Failure],
            "401 transport finished before refreshed attempt"
        );
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            1,
            "rebuilt provider may not send before its grant"
        );
        gate.allow_second.add_permits(1);
    }
    let response = tokio::time::timeout(LIMIT, task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(response.content.as_deref(), Some("runtime-oauth-ok"));
    assert_eq!(refreshed.load(Ordering::SeqCst), 1);
    assert_eq!(
        callbacks.load(Ordering::SeqCst),
        usize::from(!enabled),
        "only disabled ingress uses the legacy callback"
    );
    assert_eq!(
        gate.starts.load(Ordering::SeqCst),
        if enabled { 2 } else { 0 }
    );
    assert_eq!(
        *gate.finishes.lock().unwrap(),
        if enabled {
            vec![Feedback::Failure, Feedback::Success]
        } else {
            vec![]
        }
    );
    let wire = server.received_requests().await.unwrap();
    assert_eq!(wire.len(), 2);
    for (request, (token, account)) in wire
        .iter()
        .zip([(&old, "old-account"), (&rotated, "rotated-account")])
    {
        assert_eq!(request.url.path(), "/codex/responses");
        assert_eq!(
            request.headers["authorization"].to_str().unwrap(),
            format!("Bearer {token}")
        );
        assert_eq!(
            request.headers["chatgpt-account-id"].to_str().unwrap(),
            account
        );
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["model"], "retained-model");
    }
}

#[tokio::test]
async fn actual_openai_oauth_refresh_retains_original_bound_gate_and_rotates_wire_account() {
    actual_runtime_oauth_refresh(true).await;
}
#[tokio::test]
async fn disabled_openai_oauth_refresh_preserves_caller_callback() {
    actual_runtime_oauth_refresh(false).await;
}
