//! Loopback-only AC5 transport oracle, independent of the attempt-lifetime fixture.
//!
//! The recording port checks delivery/ownership, NOT production cooldown policy.
//! A receipt closes this fixture gate until explicitly reopened. P1 policy tests
//! own deadline merging/expiry. Header feedback remains absolute `Until`.
//! Typed SSE errors without a provider hint report `NoHint { jitter }` via
//! `AttemptPermit::throttle_without_hint`; the domain group computes fallback
//! and owns consecutive-throttle counters. Leaves never choose fallback_ms.
//! Capture receipt time immediately after `send().await`, normalize typed
//! headers, and call `feedback` before any body await. No transport/header
//! types need cross the inward port. Exact timestamp arithmetic remains the
//! normalizer's executable unit contract, independently of this recording gate.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use quecto::application::inference_attempt::{AttemptAdmission, AttemptPermit};
use quecto::domain::error::DomainError;
use quecto::domain::inference_admission::{Feedback, ThrottleFeedback};
use quecto::domain::provider::{ChatRequest, LlmProvider, StreamEvent};
use quecto::infrastructure::providers::{
    anthropic::AnthropicProvider, codex::CodexProvider, openai::OpenAiProvider,
};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{Notify, mpsc, oneshot};

pub const LIMIT: Duration = Duration::from_secs(5);
pub async fn bounded<T>(f: impl Future<Output = T>) -> T {
    tokio::time::timeout(LIMIT, f)
        .await
        .expect("loopback fixture deadline")
}
pub struct Task<T>(pub tokio::task::JoinHandle<T>);
impl<T> Drop for Task<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Leaf {
    OpenAi,
    Responses,
    OAuth,
    Anthropic,
}
impl Leaf {
    pub fn provider(self, url: &str, gate: Arc<Gate>) -> Arc<dyn LlmProvider> {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        match self {
            Self::OpenAi => Arc::new(
                OpenAiProvider::with_client("fixture".into(), Some(url.into()), client)
                    .with_attempt_admission(gate),
            ),
            Self::Responses => Arc::new(
                CodexProvider::with_api_key("fixture".into(), Some(url.into()), client)
                    .with_attempt_admission(gate),
            ),
            Self::OAuth => Arc::new(
                CodexProvider::with_client(
                    "fixture".into(),
                    "fixture-account".into(),
                    Some(url.into()),
                    client,
                )
                .with_attempt_admission(gate),
            ),
            Self::Anthropic => Arc::new(
                AnthropicProvider::with_client("fixture".into(), Some(url.into()), client)
                    .with_attempt_admission(gate),
            ),
        }
    }
    pub fn sse(
        self,
        post_text: bool,
        kind: &str,
        code: Option<&str>,
        message: &str,
    ) -> (String, String) {
        let mut error = json!({"type":kind,"message":message});
        if let Some(code) = code {
            error["code"] = code.into();
        }
        match self {
            Self::Responses | Self::OAuth => (
                format!(
                    "{}data: {}\n\n",
                    if post_text {
                        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"visible once\"}\n\n"
                    } else {
                        ""
                    },
                    json!({"type":"response.failed","response":{"status":"failed","error":error}})
                ),
                format!("Responses stream response.failed: status=failed: type={kind}: {message}"),
            ),
            Self::Anthropic => (
                format!(
                    "{}event: error\ndata: {}\n\n",
                    if post_text {
                        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"visible once\"}}\n\n"
                    } else {
                        ""
                    },
                    json!({"type":"error","error":error})
                ),
                format!("Anthropic stream error: type={kind}: {message}"),
            ),
            Self::OpenAi => {
                unreachable!("OpenAI SSE error vocabulary needs a separate parser contract")
            }
        }
    }
}

pub fn request() -> ChatRequest<'static> {
    static MESSAGES: std::sync::LazyLock<Vec<quecto::domain::message::Message>> =
        std::sync::LazyLock::new(|| {
            vec![quecto::domain::message::Message::system(
                "Fixture instructions",
            )]
        });
    ChatRequest {
        messages: &MESSAGES,
        tools: &[],
        model: "fixture-model",
        max_tokens: 32,
        temperature: 0.0,
        session_id: Some("feedback-fixture"),
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    }
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub queued: usize,
    pub grants: usize,
    pub receipts: Vec<ThrottleFeedback>,
    pub finishes: Vec<Feedback>,
    pub abandoned: usize,
    blocked: bool,
    pub parked: usize,
}
#[derive(Debug, Default, Clone)]
pub struct Gate {
    state: Arc<Mutex<Snapshot>>,
    changed: Arc<Notify>,
    // Publishing a parked acquisition must not wake that acquisition's own
    // `changed` waiter: on a current-thread runtime that becomes a busy loop.
    // Separate observer notification preserves the pre-check waiter ordering
    // needed to avoid losing a concurrent explicit reopen.
    parked_changed: Arc<Notify>,
}
impl Gate {
    pub fn snapshot(&self) -> Snapshot {
        self.state.lock().unwrap().clone()
    }
    pub fn reopen(&self) {
        self.state.lock().unwrap().blocked = false;
        self.changed.notify_waiters();
    }
    pub async fn blocked_sibling(&self) {
        bounded(async {
            loop {
                let changed = self.parked_changed.notified();
                if self.snapshot().parked > 0 {
                    return;
                }
                changed.await;
            }
        })
        .await;
    }
    pub async fn observe_receipt(&self) {
        // A deadline is an observation window, not the RED oracle. Always let
        // the caller assert the actual recorded receipts/counters afterward.
        let _ = tokio::time::timeout(Duration::from_millis(300), async {
            loop {
                let changed = self.changed.notified();
                if !self.snapshot().receipts.is_empty() {
                    return;
                }
                changed.await;
            }
        })
        .await;
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
            self.gate.state.lock().unwrap().abandoned += 1;
        }
    }
}
impl AttemptPermit for Permit {
    fn feedback(&mut self, feedback: ThrottleFeedback) {
        let mut state = self.gate.state.lock().unwrap();
        state.receipts.push(feedback);
        state.blocked = true;
        drop(state);
        self.gate.changed.notify_waiters();
    }
    fn finish(mut self: Box<Self>, feedback: Feedback) {
        self.finished = true;
        self.gate.state.lock().unwrap().finishes.push(feedback);
        self.gate.changed.notify_waiters();
    }
}
impl AttemptAdmission for Gate {
    fn acquire(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn AttemptPermit>, DomainError>> + Send + '_>>
    {
        Box::pin(async move {
            self.state.lock().unwrap().queued += 1;
            loop {
                let changed = self.changed.notified();
                {
                    let mut state = self.state.lock().unwrap();
                    // Deliberately no occupancy cap: a blocked sibling must be
                    // blocked by receipt feedback, not the stalled first body.
                    if !state.blocked {
                        state.grants += 1;
                        return Ok(Box::new(Permit {
                            gate: self.clone(),
                            finished: false,
                        }) as Box<dyn AttemptPermit>);
                    }
                    state.parked += 1;
                    self.parked_changed.notify_waiters();
                }
                changed.await;
            }
        })
    }
}

pub struct Reply {
    pub status: u16,
    pub headers: String,
    pub body: String,
    pub stalled: bool,
}
pub struct Connection {
    release: Option<oneshot::Sender<()>>,
}
impl Connection {
    pub fn release(mut self) {
        if let Some(tx) = self.release.take() {
            let _ = tx.send(());
        }
    }
}
pub struct Server {
    pub url: String,
    starts: Arc<AtomicUsize>,
    connections: mpsc::UnboundedReceiver<Connection>,
    _task: Task<()>,
}
impl Server {
    pub async fn start(reply: Reply) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let starts = Arc::new(AtomicUsize::new(0));
        let count = starts.clone();
        let (tx, connections) = mpsc::unbounded_channel();
        let reply = Arc::new(reply);
        let task = Task(tokio::spawn(async move {
            let mut tasks = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (mut socket, _) = accepted.unwrap();
                        let reply = reply.clone(); let tx = tx.clone(); let count = count.clone();
                        tasks.spawn(async move {
                            let mut header = Vec::new(); let mut byte = [0];
                            while !header.ends_with(b"\r\n\r\n") {
                                socket.read_exact(&mut byte).await.unwrap(); header.push(byte[0]);
                                assert!(header.len() < 32768, "bounded HTTP request headers");
                            }
                            let header = String::from_utf8(header).unwrap();
                            assert!(header.starts_with("POST "), "physical inference POST");
                            let length: usize = header.lines().find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse().unwrap())
                            }).unwrap();
                            assert!(length < 1_048_576);
                            socket.read_exact(&mut vec![0; length]).await.unwrap();
                            count.fetch_add(1, Ordering::SeqCst);
                            socket.write_all(format!("HTTP/1.1 {} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n{}\r\n", reply.status, reply.body.len(), reply.headers).as_bytes()).await.unwrap();
                            let (release, released) = oneshot::channel();
                            tx.send(Connection { release: Some(release) }).unwrap_or_else(|_| panic!("live test receiver"));
                            if reply.stalled && released.await.is_err() { return; }
                            let _ = socket.write_all(reply.body.as_bytes()).await;
                        });
                    }
                    result = tasks.join_next(), if !tasks.is_empty() => { result.unwrap().unwrap(); }
                }
            }
        }));
        Self {
            url,
            starts,
            connections,
            _task: task,
        }
    }
    pub fn starts(&self) -> usize {
        self.starts.load(Ordering::SeqCst)
    }
    pub async fn connection(&mut self) -> Connection {
        bounded(self.connections.recv())
            .await
            .expect("real HTTP headers sent")
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Transcript {
    pub text: Vec<String>,
    pub errors: Vec<String>,
    pub done: usize,
}
pub async fn stream(provider: &dyn LlmProvider) -> Transcript {
    let mut rx = provider.chat_stream_incremental(request()).await;
    let mut output = Transcript::default();
    while let Some(event) = rx.recv().await {
        match event {
            StreamEvent::TextDelta(text) => output.text.push(text),
            StreamEvent::Error(error) => output.errors.push(error),
            StreamEvent::Done(_) => output.done += 1,
            other => panic!("unexpected fixture event {other:?}"),
        }
    }
    output
}
