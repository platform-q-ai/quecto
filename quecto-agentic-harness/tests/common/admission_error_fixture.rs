//! Exact enabled/disabled leaf compatibility oracle, using only loopback.
//! The capability always grants: this is not another admission policy or scope.
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};

use quecto::{
    application::inference_attempt::{AttemptAdmission, AttemptPermit},
    domain::{
        error::DomainError,
        inference_admission::{Feedback, ThrottleFeedback},
        message::LlmResponse,
        provider::{ChatRequest, LlmProvider, StreamEvent},
        provider_error::{ProviderErrorClass, classify_provider_error, provider_http_status},
    },
    infrastructure::providers::{
        anthropic::AnthropicProvider, codex::CodexProvider, openai::OpenAiProvider,
    },
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpSocket},
};

pub async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), future)
        .await
        .expect("loopback compatibility deadline")
}

#[derive(Debug, Default, Clone)]
pub struct Snapshot {
    pub grants: usize,
    pub finishes: Vec<Feedback>,
    pub abandoned: usize,
}
#[derive(Debug, Default, Clone)]
pub struct Gate(Arc<Mutex<Snapshot>>);
impl Gate {
    pub fn snapshot(&self) -> Snapshot {
        self.0.lock().unwrap().clone()
    }
    pub fn assert_one(&self) {
        oracle::permit(&self.snapshot());
    }
}
#[derive(Debug)]
struct Permit {
    gate: Gate,
    finished: bool,
}
impl Drop for Permit {
    fn drop(&mut self) {
        if !self.finished {
            self.gate.0.lock().unwrap().abandoned += 1;
        }
    }
}
impl AttemptPermit for Permit {
    fn feedback(&mut self, _: ThrottleFeedback) {}
    fn finish(mut self: Box<Self>, feedback: Feedback) {
        self.finished = true;
        self.gate.0.lock().unwrap().finishes.push(feedback);
    }
}
impl AttemptAdmission for Gate {
    fn acquire(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn AttemptPermit>, DomainError>> + Send + '_>>
    {
        Box::pin(async move {
            self.0.lock().unwrap().grants += 1;
            Ok(Box::new(Permit {
                gate: self.clone(),
                finished: false,
            }) as Box<dyn AttemptPermit>)
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Leaf {
    OpenAi,
    Anthropic,
    Responses,
    OAuth,
}
#[derive(Clone, Copy, Debug)]
pub enum Surface {
    Chat,
    Assembled,
    Incremental,
}
impl Leaf {
    pub fn provider(self, url: &str, gate: Option<Arc<Gate>>) -> Box<dyn LlmProvider> {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let safe_client = quecto::infrastructure::providers::SingleAttemptClient::build(
            reqwest::Client::builder().no_proxy(),
        )
        .unwrap();
        macro_rules! bind {
            ($provider:expr) => {{
                let provider = $provider;
                Box::new(match gate {
                    Some(gate) => provider.with_attempt_admission(gate, safe_client),
                    None => provider,
                })
            }};
        }
        match self {
            Self::OpenAi => bind!(OpenAiProvider::with_client(
                "fixture".into(),
                Some(url.into()),
                client
            )),
            Self::Anthropic => bind!(AnthropicProvider::with_client(
                "fixture".into(),
                Some(url.into()),
                client
            )),
            Self::Responses => bind!(CodexProvider::with_api_key(
                "fixture".into(),
                Some(url.into()),
                client
            )),
            Self::OAuth => bind!(CodexProvider::with_client(
                "fixture".into(),
                "fixture-account".into(),
                Some(url.into()),
                client
            )),
        }
    }
    pub fn vendor(self) -> &'static str {
        match self {
            Self::OpenAi => "OpenAI",
            Self::Anthropic => "Anthropic",
            Self::Responses | Self::OAuth => "Codex",
        }
    }
    fn endpoint(self) -> &'static str {
        match self {
            Self::Anthropic => "/v1/messages",
            Self::Responses => "/responses",
            Self::OAuth => "/codex/responses",
            Self::OpenAi => "/chat/completions",
        }
    }
    pub fn delta(self, text: &str) -> String {
        match self {
            Self::Anthropic => format!(
                "event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\"{text}\"}}}}"
            ),
            Self::Responses | Self::OAuth => {
                format!("data: {{\"type\":\"response.output_text.delta\",\"delta\":\"{text}\"}}")
            }
            Self::OpenAi => unreachable!(),
        }
    }
    pub fn terminal(self) -> &'static str {
        match self {
            Self::Anthropic => "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
            Self::Responses | Self::OAuth => {
                "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n\n"
            }
            Self::OpenAi => unreachable!(),
        }
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
        messages: &MESSAGES,
        tools: &[],
        model: "fixture-model",
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub display: String,
    pub class: ProviderErrorClass,
    pub status: Option<u16>,
    pub retryable: bool,
}
impl Error {
    fn new(error: DomainError) -> Self {
        let class = classify_provider_error(&error);
        Self {
            display: error.to_string(),
            status: provider_http_status(&error),
            retryable: class.is_retryable(),
            class,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observation {
    Response {
        content: Option<String>,
        full: String,
    },
    Error(Error),
    Events(Vec<Event>),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Error { raw: String, classified: Error },
    Other(String),
}
fn response(response: LlmResponse) -> Observation {
    // LlmResponse has no PartialEq/Serialize. Debug covers every response field,
    // including usage, stop reason, tool calls and thinking blocks, without loss.
    Observation::Response {
        full: format!("{response:?}"),
        content: response.content,
    }
}
pub async fn invoke(provider: &dyn LlmProvider, surface: Surface) -> Observation {
    match surface {
        Surface::Chat => provider
            .chat(request())
            .await
            .map(response)
            .unwrap_or_else(|e| Observation::Error(Error::new(e))),
        Surface::Assembled => provider
            .chat_stream(request())
            .await
            .map(response)
            .unwrap_or_else(|e| Observation::Error(Error::new(e))),
        Surface::Incremental => {
            let mut rx = provider.chat_stream_incremental(request()).await;
            let mut events = Vec::new();
            while let Some(event) = rx.recv().await {
                events.push(match event {
                    StreamEvent::Error(raw) => Event::Error {
                        classified: Error::new(DomainError::Provider(raw.clone())),
                        raw,
                    },
                    other => Event::Other(format!("{other:?}")),
                });
            }
            Observation::Events(events)
        }
    }
}

pub struct Reply {
    pub status: u16,
    pub body: String,
    pub missing_bytes: usize,
}
struct Task(tokio::task::JoinHandle<Vec<String>>);
impl Drop for Task {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Two requests share the same listener, URL, response bytes and HTTP framing.
/// Reading the entire request first avoids an accidental TCP reset on close.
pub async fn compare(leaf: Leaf, surface: Surface, reply: Reply) -> (Observation, Observation) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let gate = Arc::new(Gate::default());
    let observed_gate = gate.clone();
    let mut server = Task(tokio::spawn(async move {
        let mut paths = Vec::new();
        for index in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut headers = Vec::new();
            while !headers.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                socket.read_exact(&mut byte).await.unwrap();
                headers.push(byte[0]);
                oracle::request_size(headers.len(), 32768);
            }
            let headers = String::from_utf8(headers).unwrap();
            let path = headers.lines().next().unwrap().to_owned();
            oracle::post(&path);
            paths.push(path);
            let length: usize = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().unwrap())
                })
                .unwrap();
            oracle::request_size(length, 1_048_576);
            socket.read_exact(&mut vec![0; length]).await.unwrap();
            oracle::grants_at_send(observed_gate.snapshot().grants, index);
            let headers = format!(
                "HTTP/1.1 {} Fixture\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                reply.status,
                reply.body.len() + reply.missing_bytes
            );
            socket.write_all(headers.as_bytes()).await.unwrap();
            socket.write_all(reply.body.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        }
        paths
    }));
    let disabled = bounded(invoke(leaf.provider(&url, None).as_ref(), surface)).await;
    oracle::disabled_grants(gate.snapshot().grants);
    let enabled = bounded(invoke(
        leaf.provider(&url, Some(gate.clone())).as_ref(),
        surface,
    ))
    .await;
    let paths = bounded(&mut server.0).await.unwrap();
    oracle::paths(&paths, leaf.endpoint());
    gate.assert_one();
    (disabled, enabled)
}

/// A bound but non-listening TCP socket reserves one port throughout both
/// refused attempts, without a release/rebind race or external connection.
pub async fn compare_refused(leaf: Leaf, surface: Surface) -> (Observation, Observation, String) {
    let socket = TcpSocket::new_v4().unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let url = format!("http://{}", socket.local_addr().unwrap());
    let gate = Arc::new(Gate::default());
    let disabled = bounded(invoke(leaf.provider(&url, None).as_ref(), surface)).await;
    let enabled = bounded(invoke(
        leaf.provider(&url, Some(gate.clone())).as_ref(),
        surface,
    ))
    .await;
    gate.assert_one();
    drop(socket);
    (disabled, enabled, url)
}

pub fn assert_parity(disabled: &Observation, enabled: &Observation) {
    // Print classifier independently so an exact-string failure does not hide
    // the retryability regression in the RED evidence.
    eprintln!("disabled: {disabled:?}\nenabled: {enabled:?}");
    assert_eq!(
        enabled, disabled,
        "admission must preserve exact leaf errors, classifier and response fields"
    );
}

/// Exact same assertion entrypoints for live fixture and sensitivity tests.
/// No transfer claim from a different suite: each invariant is falsified here.
pub mod oracle {
    use super::Snapshot;
    pub fn permit(state: &Snapshot) {
        assert_eq!(state.grants, 1, "F01 one enabled grant");
        assert_eq!(state.finishes.len(), 1, "F02 one transport finish");
        assert_eq!(state.abandoned, 0, "F03 no abandoned permit");
    }
    pub fn request_size(actual: usize, limit: usize) {
        assert!(actual < limit, "F04/F05 bounded request headers/body");
    }
    pub fn post(path: &str) {
        assert!(path.starts_with("POST "), "F06 inference POST");
    }
    pub fn grants_at_send(actual: usize, index: usize) {
        assert_eq!(actual, index, "F07/F08 grant precedes enabled POST only");
    }
    pub fn disabled_grants(actual: usize) {
        assert_eq!(actual, 0, "F09 disabled invocation bypasses capability");
    }
    pub fn paths(paths: &[String], endpoint: &str) {
        assert_eq!(paths.len(), 2, "F10 exactly two observed requests");
        assert_eq!(
            paths[0],
            format!("POST {endpoint} HTTP/1.1"),
            "F11 expected leaf endpoint"
        );
        assert_eq!(paths[0], paths[1], "F12 same endpoint across modes");
    }
}
