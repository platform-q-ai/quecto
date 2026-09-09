//! A single permit authorizes one wire send, not a reqwest redirect replay.
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use quecto::application::ports::{AttemptAdmission, AttemptPermit};
use quecto::domain::error::DomainError;
use quecto::domain::inference_admission::{Feedback, ThrottleFeedback};
use quecto::domain::provider::{ChatRequest, LlmProvider, StreamEvent};
use quecto::infrastructure::providers::{
    SingleAttemptClient, anthropic::AnthropicProvider, codex::CodexProvider, openai::OpenAiProvider,
};
#[path = "common/admission_redirect_oracle.rs"]
mod oracle;

use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

#[derive(Debug, Default)]
struct Gate {
    grants: Mutex<usize>,
    finishes: Arc<Mutex<Vec<Feedback>>>,
}
#[derive(Debug)]
struct Permit(Arc<Mutex<Vec<Feedback>>>);
impl AttemptAdmission for Gate {
    fn acquire(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn AttemptPermit>, DomainError>> + Send + '_>>
    {
        Box::pin(async {
            *self.grants.lock().unwrap() += 1;
            Ok(Box::new(Permit(self.finishes.clone())) as Box<dyn AttemptPermit>)
        })
    }
}
impl AttemptPermit for Permit {
    fn feedback(&mut self, _: ThrottleFeedback) {}
    fn finish(self: Box<Self>, feedback: Feedback) {
        self.0.lock().unwrap().push(feedback);
    }
}
fn request() -> ChatRequest<'static> {
    static MESSAGES: std::sync::LazyLock<Vec<quecto::domain::message::Message>> =
        std::sync::LazyLock::new(|| {
            vec![quecto::domain::message::Message::system(
                "Fixture instructions",
            )]
        });
    ChatRequest {
        trace: None,
        admission: None,
        model: "fixture",
        messages: &MESSAGES,
        tools: &[],
        max_tokens: 1,
        temperature: 0.0,
        thinking_level: None,
        effort: None,
        tool_choice: None,
        metadata: None,
        session_id: None,
        cancel_flag: None,
    }
}

async fn check(status: u16, leaf: usize, surface: usize, enabled: bool) {
    let server = MockServer::start().await;
    let endpoint = [
        "/chat/completions",
        "/responses",
        "/codex/responses",
        "/v1/messages",
    ][leaf];
    Mock::given(method("POST"))
        .and(path(endpoint))
        .respond_with(ResponseTemplate::new(status).insert_header("Location", "/redirect-target"))
        .mount(&server)
        .await;
    // A definitive completed response, rather than a quiet-time absence oracle.
    Mock::given(method("POST"))
        .and(path("/redirect-target"))
        .respond_with(ResponseTemplate::new(400).set_body_string("redirect replay reached target"))
        .mount(&server)
        .await;
    let gate = Arc::new(Gate::default());
    // Both clients use the caller's transport recipe; safe construction changes
    // redirect/retry policy only, never silently discards transport settings.
    let configured_builder = || {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("x-fixture-transport", "configured".parse().unwrap());
        reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(3))
            .default_headers(headers)
    };
    let client = configured_builder().build().unwrap();
    macro_rules! bind {
        ($provider:expr) => {{
            let provider = $provider;
            let provider = if enabled {
                provider.with_attempt_admission(
                    gate.clone(),
                    SingleAttemptClient::build(configured_builder()).unwrap(),
                )
            } else {
                provider
            };
            Arc::new(provider) as Arc<dyn LlmProvider>
        }};
    }
    let provider = match leaf {
        0 => bind!(OpenAiProvider::with_client(
            "fixture".into(),
            Some(server.uri()),
            client
        )),
        1 => bind!(CodexProvider::with_api_key(
            "fixture".into(),
            Some(server.uri()),
            client
        )),
        2 => bind!(CodexProvider::with_client(
            "fixture".into(),
            "account".into(),
            Some(server.uri()),
            client
        )),
        3 => bind!(AnthropicProvider::with_client(
            "fixture".into(),
            Some(server.uri()),
            client
        )),
        _ => unreachable!(),
    };
    let error = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        match surface {
            0 => oracle::failure(provider.chat(request()).await),
            1 => oracle::failure(provider.chat_stream(request()).await),
            2 => {
                let mut rx = provider.chat_stream_incremental(request()).await;
                let mut error = None;
                let mut done = false;
                while let Some(event) = rx.recv().await {
                    match event {
                        StreamEvent::Error(message) => error = Some(message),
                        StreamEvent::Done(_) => done = true,
                        _ => {}
                    }
                }
                oracle::terminal(error, done)
            }
            _ => unreachable!(),
        }
    })
    .await
    .expect("bounded loopback request");
    let requests = server.received_requests().await.unwrap();
    oracle::sends(requests.len(), if enabled { 1 } else { 2 });
    oracle::endpoint(requests[0].url.path(), endpoint);
    oracle::header(requests[0].headers.get("x-fixture-transport"));
    oracle::grants(*gate.grants.lock().unwrap(), usize::from(enabled));
    if enabled {
        oracle::status(&error, status);
        oracle::finishes(&gate.finishes.lock().unwrap(), &[Feedback::Failure]);
    } else {
        oracle::endpoint(requests[1].url.path(), "/redirect-target");
        oracle::body(&requests[0].body, &requests[1].body);
        oracle::finishes(&gate.finishes.lock().unwrap(), &[]);
    }
}

macro_rules! enabled_case {
    ($name:ident, $status:literal, $leaf:literal, $surface:literal) => {
        #[tokio::test]
        async fn $name() {
            check($status, $leaf, $surface, true).await;
        }
    };
}
enabled_case!(enabled_307_openai_chat, 307, 0, 0);
enabled_case!(enabled_307_openai_assembled, 307, 0, 1);
enabled_case!(enabled_307_openai_incremental, 307, 0, 2);
enabled_case!(enabled_307_responses_chat, 307, 1, 0);
enabled_case!(enabled_307_responses_assembled, 307, 1, 1);
enabled_case!(enabled_307_responses_incremental, 307, 1, 2);
enabled_case!(enabled_307_oauth_chat, 307, 2, 0);
enabled_case!(enabled_307_oauth_assembled, 307, 2, 1);
enabled_case!(enabled_307_oauth_incremental, 307, 2, 2);
enabled_case!(enabled_307_anthropic_chat, 307, 3, 0);
enabled_case!(enabled_307_anthropic_assembled, 307, 3, 1);
enabled_case!(enabled_307_anthropic_incremental, 307, 3, 2);
enabled_case!(enabled_308_openai_chat, 308, 0, 0);
enabled_case!(enabled_308_openai_assembled, 308, 0, 1);
enabled_case!(enabled_308_openai_incremental, 308, 0, 2);
enabled_case!(enabled_308_responses_chat, 308, 1, 0);
enabled_case!(enabled_308_responses_assembled, 308, 1, 1);
enabled_case!(enabled_308_responses_incremental, 308, 1, 2);
enabled_case!(enabled_308_oauth_chat, 308, 2, 0);
enabled_case!(enabled_308_oauth_assembled, 308, 2, 1);
enabled_case!(enabled_308_oauth_incremental, 308, 2, 2);
enabled_case!(enabled_308_anthropic_chat, 308, 3, 0);
enabled_case!(enabled_308_anthropic_assembled, 308, 3, 1);
enabled_case!(enabled_308_anthropic_incremental, 308, 3, 2);

#[tokio::test]
async fn disabled_injected_default_client_still_follows_307_and_308() {
    for status in [307, 308] {
        for leaf in 0..4 {
            for surface in 0..3 {
                check(status, leaf, surface, false).await;
            }
        }
    }
}

#[tokio::test]
async fn enabled_overrides_configured_reqwest_retries_without_losing_headers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503).set_body_string("unavailable"))
        .mount(&server)
        .await;
    let builder = || {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("x-fixture-transport", "configured".parse().unwrap());
        reqwest::Client::builder()
            .no_proxy()
            .default_headers(headers)
            .retry(
                reqwest::retry::for_host("127.0.0.1")
                    .no_budget()
                    .max_retries_per_request(2)
                    .classify_fn(|response| response.retryable()),
            )
    };
    let gate = Arc::new(Gate::default());
    let provider = OpenAiProvider::with_client(
        "fixture".into(),
        Some(server.uri()),
        builder().build().unwrap(),
    );
    let enabled = provider
        .clone()
        .with_attempt_admission(gate.clone(), SingleAttemptClient::build(builder()).unwrap());
    oracle::status(&oracle::failure(enabled.chat(request()).await), 503);
    let requests = server.received_requests().await.unwrap();
    oracle::sends(requests.len(), 1);
    oracle::header(requests[0].headers.get("x-fixture-transport"));
    oracle::grants(*gate.grants.lock().unwrap(), 1);
    oracle::finishes(&gate.finishes.lock().unwrap(), &[Feedback::Failure]);
    server.reset().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503).set_body_string("unavailable"))
        .mount(&server)
        .await;
    oracle::failure(provider.chat(request()).await);
    oracle::sends(server.received_requests().await.unwrap().len(), 3);
}
