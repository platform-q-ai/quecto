//! Proposed inward public API (parent owns production implementation):
//!
//! ```ignore
//! // application::inference_attempt
//! pub trait AttemptAdmission: Debug + Send + Sync {
//!     fn acquire(&self) -> Pin<Box<dyn Future<Output =
//!         Result<Box<dyn AttemptPermit>, DomainError>> + Send + '_>>;
//! }
//! pub trait AttemptPermit: Debug + Send {
//!     fn deadline_expired(&self) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>;
//!     fn feedback(&mut self, feedback: ThrottleFeedback);
//!     fn finish(self: Box<Self>, feedback: Feedback);
//! }
//! // All three concrete leaves (including both Responses auth constructors):
//! pub fn with_attempt_admission(self, gate: Arc<dyn AttemptAdmission>,
//!     client: SingleAttemptClient) -> Self;
//! ```
//!
//! The gate is already bound to a trusted scope and endpoint/account alias.
//! `finish` acknowledges owned HTTP future/response/pump destruction, NOT return
//! of a receiver, cancellation intent, or requesting JoinHandle::abort. Dropping
//! an uncertain permit never frees capacity. No transport types leak inward.
//! This fake is deliberately NOT an LlmProvider decorator or policy substitute.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use quecto::application::inference_attempt::{AttemptAdmission, AttemptPermit};
use quecto::domain::error::DomainError;
use quecto::domain::inference_admission::{Feedback, ThrottleFeedback};
use quecto::domain::provider::{CancelFlag, ChatRequest, LlmProvider, StreamEvent};
use quecto::infrastructure::providers::{
    anthropic::AnthropicProvider, codex::CodexProvider, openai::OpenAiProvider,
};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Notify, mpsc, oneshot};

pub const LIMIT: Duration = Duration::from_secs(5);

pub async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(LIMIT, future)
        .await
        .expect("bounded fixture deadline")
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
    CodexOAuth,
    Anthropic,
}
#[derive(Clone, Copy, Debug)]
pub enum Surface {
    Chat,
    Stream,
    Incremental,
}

impl Leaf {
    pub fn provider(self, url: &str, gate: Arc<Gate>) -> Arc<dyn LlmProvider> {
        // Bypass environment proxies; this suite must never leave loopback.
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let safe_client = quecto::infrastructure::providers::SingleAttemptClient::build(
            reqwest::Client::builder().no_proxy(),
        )
        .unwrap();
        match self {
            Self::OpenAi => Arc::new(
                OpenAiProvider::with_client("fixture".into(), Some(url.into()), client)
                    .with_attempt_admission(gate, safe_client),
            ),
            Self::Responses => Arc::new(
                CodexProvider::with_api_key("fixture".into(), Some(url.into()), client)
                    .with_attempt_admission(gate, safe_client),
            ),
            Self::CodexOAuth => Arc::new(
                CodexProvider::with_client(
                    "fixture".into(),
                    "account".into(),
                    Some(url.into()),
                    client,
                )
                .with_attempt_admission(gate, safe_client),
            ),
            Self::Anthropic => Arc::new(
                AnthropicProvider::with_client("fixture".into(), Some(url.into()), client)
                    .with_attempt_admission(gate, safe_client),
            ),
        }
    }

    fn body(self, surface: Surface, count: usize) -> (String, String, &'static str) {
        let text = (0..count).map(|i| format!("{i},")).collect::<String>();
        if matches!(surface, Surface::Chat) {
            match self {
                Self::OpenAi => return (json!({"choices":[{"message":{"content":text},"finish_reason":"stop"}]}).to_string(), String::new(), "application/json"),
                Self::Anthropic => return (json!({"content":[{"type":"text","text":text}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}).to_string(), String::new(), "application/json"),
                _ => {}
            }
        }
        let delta = (0..count).map(|i| match self {
            Self::OpenAi => format!("data: {}\n\n", json!({"choices":[{"delta":{"content":format!("{i},")}}]})),
            Self::Responses | Self::CodexOAuth => format!("data: {}\n\n", json!({"type":"response.output_text.delta","delta":format!("{i},")})),
            Self::Anthropic => format!("event: content_block_delta\ndata: {}\n\n", json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":format!("{i},")}})),
        }).collect::<String>();
        let terminal = match self {
            Self::OpenAi => "data: [DONE]\n\n",
            Self::Responses | Self::CodexOAuth => {
                "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n\n"
            }
            Self::Anthropic => "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        };
        (delta, terminal.into(), "text/event-stream")
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
        max_tokens: 64,
        temperature: 0.0,
        session_id: Some("admission-attempt"),
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    }
}

pub async fn invoke(provider: &dyn LlmProvider, surface: Surface) -> Result<String, String> {
    match surface {
        Surface::Chat => provider
            .chat(request())
            .await
            .map(|r| r.content.unwrap_or_default())
            .map_err(|e| e.to_string()),
        Surface::Stream => provider
            .chat_stream(request())
            .await
            .map(|r| r.content.unwrap_or_default())
            .map_err(|e| e.to_string()),
        Surface::Incremental => {
            let mut rx = provider.chat_stream_incremental(request()).await;
            let mut deltas = String::new();
            let mut done = None;
            while let Some(event) = rx.recv().await {
                match event {
                    StreamEvent::TextDelta(text) => deltas.push_str(&text),
                    StreamEvent::Done(response) => {
                        oracle::done_count(done.is_some());
                        done = Some(response.content.unwrap_or_default());
                    }
                    StreamEvent::Error(error) => return Err(error),
                    other => panic!("unexpected fixture event {other:?}"),
                }
            }
            let done = oracle::terminal(done);
            oracle::text(&deltas, &done);
            Ok(done)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Queued(usize),
    Granted(usize),
    QueueCancelled(usize),
    QueueExpired(usize),
    ActiveExpired(usize),
    RawStart(usize),
    Finished(usize),
    Abandoned(usize),
    PeerEof(usize),
}

#[derive(Debug, Default)]
struct State {
    events: Vec<Event>,
    next: usize,
    active: usize,
    held: bool,
    cancel_at_grant: Option<CancelFlag>,
    queue_expired: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Gate {
    state: Arc<Mutex<State>>,
    changed: Arc<Notify>,
}
impl Gate {
    pub fn new(held: bool) -> Arc<Self> {
        let gate = Arc::new(Self::default());
        gate.state.lock().unwrap().held = held;
        gate
    }
    pub fn events(&self) -> Vec<Event> {
        self.state.lock().unwrap().events.clone()
    }
    pub fn active(&self) -> usize {
        self.state.lock().unwrap().active
    }
    pub fn open(&self) {
        self.state.lock().unwrap().held = false;
        self.changed.notify_waiters();
    }
    pub fn cancel_on_grant(&self, flag: CancelFlag) {
        self.state.lock().unwrap().cancel_at_grant = Some(flag);
    }
    pub fn expire_queue(&self) {
        self.state.lock().unwrap().queue_expired = true;
        self.changed.notify_waiters();
    }
    pub fn expire_active(&self, id: usize) {
        self.record(Event::ActiveExpired(id));
    }
    pub async fn wait(&self, predicate: impl Fn(&[Event]) -> bool) {
        bounded(async {
            loop {
                let changed = self.changed.notified();
                let events = self.events();
                oracle::granted_before_send(&events);
                if predicate(&events) {
                    return;
                }
                changed.await;
            }
        })
        .await;
    }
    pub async fn wait_event(&self, event: Event) {
        self.wait(|events| events.contains(&event)).await;
        oracle::present(&self.events(), event);
    }
    pub fn record(&self, event: Event) {
        self.state.lock().unwrap().events.push(event);
        self.changed.notify_waiters();
    }
    pub fn assert_exact(&self, attempts: usize) {
        let events = self.events();
        oracle::exact(&events, attempts);
    }
}

struct Queued {
    gate: Gate,
    id: usize,
    granted: bool,
}
impl Drop for Queued {
    fn drop(&mut self) {
        if !self.granted {
            self.gate.record(Event::QueueCancelled(self.id));
        }
    }
}
#[derive(Debug)]
struct Permit {
    gate: Gate,
    id: usize,
    finished: bool,
}
impl Drop for Permit {
    fn drop(&mut self) {
        if !self.finished {
            self.gate.record(Event::Abandoned(self.id));
        }
    }
}
impl AttemptPermit for Permit {
    fn deadline_expired(&self) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        let gate = self.gate.clone();
        let id = self.id;
        Box::pin(async move {
            loop {
                let changed = gate.changed.notified();
                if gate.events().contains(&Event::ActiveExpired(id)) {
                    return;
                }
                changed.await;
            }
        })
    }
    fn feedback(&mut self, _: ThrottleFeedback) {}
    fn finish(mut self: Box<Self>, _: Feedback) {
        self.finished = true;
        let mut state = self.gate.state.lock().unwrap();
        state.active -= 1;
        state.events.push(Event::Finished(self.id));
        drop(state);
        self.gate.changed.notify_waiters();
    }
}
impl AttemptAdmission for Gate {
    fn acquire(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn AttemptPermit>, DomainError>> + Send + '_>>
    {
        Box::pin(async move {
            let id = {
                let mut state = self.state.lock().unwrap();
                let id = state.next;
                state.next += 1;
                state.events.push(Event::Queued(id));
                id
            };
            self.changed.notify_waiters();
            let mut queued = Queued {
                gate: self.clone(),
                id,
                granted: false,
            };
            loop {
                let changed = self.changed.notified();
                {
                    let mut state = self.state.lock().unwrap();
                    if state.queue_expired {
                        state.events.push(Event::QueueExpired(id));
                        queued.granted = true; // terminal rejection, not caller cancellation
                        self.changed.notify_waiters();
                        return Err(DomainError::Provider(
                            "admission queue deadline expired".into(),
                        ));
                    }
                    if !state.held && state.active == 0 {
                        state.active += 1;
                        state.events.push(Event::Granted(id));
                        if let Some(flag) = state.cancel_at_grant.take() {
                            flag.cancel();
                        }
                        queued.granted = true;
                        self.changed.notify_waiters();
                        return Ok(Box::new(Permit {
                            gate: self.clone(),
                            id,
                            finished: false,
                        }) as Box<dyn AttemptPermit>);
                    }
                }
                changed.await;
            }
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Pause {
    None,
    Headers,
    Body,
}
#[derive(Debug)]
enum Command {
    Finish,
    Eof,
}
pub struct Connection {
    command: Option<oneshot::Sender<Command>>,
    done: oneshot::Receiver<()>,
}
impl Connection {
    pub async fn finish(mut self) {
        self.command
            .take()
            .unwrap()
            .send(Command::Finish)
            .expect("live server");
        bounded(&mut self.done).await.unwrap();
    }
    pub async fn eof(mut self) {
        self.command
            .take()
            .unwrap()
            .send(Command::Eof)
            .expect("live server");
        bounded(&mut self.done).await.unwrap();
    }
}

pub struct Server {
    pub url: String,
    connections: mpsc::Receiver<Connection>,
    task: Task<()>,
}
impl Server {
    pub async fn start(
        leaf: Leaf,
        surface: Surface,
        count: usize,
        pause: Pause,
        gate: Arc<Gate>,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, connections) = mpsc::channel(8);
        let task = Task(tokio::spawn(async move {
            let mut tasks = tokio::task::JoinSet::new();
            let mut serial = 0;
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (socket, _) = accepted.unwrap();
                        let gate = gate.clone(); let tx = tx.clone();
                        let id = serial; serial += 1;
                        tasks.spawn(async move { bounded(serve(socket, ReplyPlan { leaf, surface, count, pause }, gate, id, tx)).await });
                    }
                    result = tasks.join_next(), if !tasks.is_empty() => { result.unwrap().unwrap(); }
                }
            }
        }));
        Self {
            url,
            connections,
            task,
        }
    }
    pub async fn connection(&mut self) -> Connection {
        bounded(self.connections.recv())
            .await
            .expect("raw connection observed")
    }
    pub async fn shutdown(mut self) {
        self.task.0.abort();
        let _ = bounded(&mut self.task.0).await;
    }
}

/// Immutable response shape shared by each accepted fixture connection.
struct ReplyPlan {
    leaf: Leaf,
    surface: Surface,
    count: usize,
    pause: Pause,
}

async fn serve(
    mut socket: TcpStream,
    reply: ReplyPlan,
    gate: Arc<Gate>,
    id: usize,
    connections: mpsc::Sender<Connection>,
) {
    let ReplyPlan {
        leaf,
        surface,
        count,
        pause,
    } = reply;
    let mut headers = Vec::new();
    let mut byte = [0];
    loop {
        socket.read_exact(&mut byte).await.unwrap();
        headers.push(byte[0]);
        oracle::request_size(headers.len(), 32_768);
        if headers.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let headers = String::from_utf8(headers).unwrap();
    oracle::request_method(&headers);
    let length: usize = headers
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().unwrap())
        })
        .unwrap();
    oracle::request_size(length, 1_048_576);
    socket.read_exact(&mut vec![0; length]).await.unwrap();
    gate.record(Event::RawStart(id));
    let (prefix, suffix, content_type) = leaf.body(surface, count);
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        prefix.len() + suffix.len()
    );
    let (command, commanded) = oneshot::channel();
    let (done, completed) = oneshot::channel();
    if !matches!(pause, Pause::Headers) {
        socket.write_all(head.as_bytes()).await.unwrap();
        if !matches!(surface, Surface::Chat)
            || matches!(leaf, Leaf::Responses | Leaf::CodexOAuth)
            || matches!(pause, Pause::None)
        {
            socket.write_all(prefix.as_bytes()).await.unwrap();
        }
    }
    connections
        .send(Connection {
            command: Some(command),
            done: completed,
        })
        .await
        .unwrap();
    if matches!(pause, Pause::None) {
        socket.write_all(suffix.as_bytes()).await.unwrap();
    } else {
        match commanded.await {
            Ok(Command::Finish) => {
                if matches!(pause, Pause::Headers) {
                    socket.write_all(head.as_bytes()).await.unwrap();
                    socket.write_all(prefix.as_bytes()).await.unwrap();
                } else if matches!(surface, Surface::Chat)
                    && matches!(leaf, Leaf::OpenAi | Leaf::Anthropic)
                {
                    socket.write_all(prefix.as_bytes()).await.unwrap();
                }
                socket.write_all(suffix.as_bytes()).await.unwrap();
            }
            Ok(Command::Eof) => {
                // Independent TCP corroboration only, not a remote-computation claim.
                oracle::eof(socket.read(&mut byte).await.unwrap());
                gate.record(Event::PeerEof(id));
            }
            Err(_) => return, // enclosing assertion failed; RAII bounds cleanup.
        }
    }
    drop(socket);
    let _ = done.send(());
}

/// Shared observation oracles: both real leaf tests and synthetic counterexamples
/// call these. No provider, gate state, or transport enforcement lives here.
pub mod oracle {
    use super::{Event, StreamEvent};

    pub fn request_size(actual: usize, exclusive_limit: usize) {
        assert!(actual < exclusive_limit, "bounded fixture request");
    }
    pub fn request_method(headers: &str) {
        assert!(
            headers.starts_with("POST "),
            "actual inference HTTP request"
        );
    }
    pub fn active(actual: usize, expected: usize) {
        assert_eq!(actual, expected, "transport-owned occupancy");
    }
    pub fn trace(actual: &[Event], expected: &[Event]) {
        assert_eq!(actual, expected, "exact lifecycle trace");
    }
    pub fn present(events: &[Event], event: Event) {
        assert!(
            events.contains(&event),
            "required lifecycle event {event:?}: {events:?}"
        );
    }
    pub fn absent(events: &[Event], event: Event) {
        assert!(
            !events.contains(&event),
            "premature lifecycle event {event:?}: {events:?}"
        );
    }
    pub fn granted_before_send(events: &[Event]) {
        for (position, event) in events.iter().enumerate() {
            if let Event::RawStart(id) = event {
                assert!(
                    events[..position].contains(&Event::Granted(*id)),
                    "raw HTTP send without preceding grant: {events:?}"
                );
            }
        }
    }
    pub fn exact(events: &[Event], attempts: usize) {
        let grants = events
            .iter()
            .filter(|e| matches!(e, Event::Granted(_)))
            .count();
        let starts = events
            .iter()
            .filter(|e| matches!(e, Event::RawStart(_)))
            .count();
        assert_eq!(grants, attempts, "exact grant count: {events:?}");
        assert_eq!(starts, attempts, "exact physical request count: {events:?}");
        for id in 0..attempts {
            let grant = events
                .iter()
                .position(|e| *e == Event::Granted(id))
                .unwrap();
            let start = events
                .iter()
                .position(|e| *e == Event::RawStart(id))
                .unwrap();
            assert!(
                grant < start,
                "grant must precede raw HTTP start: {events:?}"
            );
        }
        assert!(
            !events.iter().any(|e| matches!(e, Event::Abandoned(_))),
            "unacknowledged transport: {events:?}"
        );
    }
    pub fn replacement_after_finish(events: &[Event]) {
        let finish = events
            .iter()
            .position(|e| *e == Event::Finished(0))
            .expect("original owner finishes");
        let grant = events
            .iter()
            .position(|e| *e == Event::Granted(1))
            .expect("replacement admitted");
        assert!(
            finish < grant,
            "replacement cannot precede original owned shutdown"
        );
    }
    pub fn capacity(actual: usize, expected: usize) {
        assert_eq!(actual, expected, "bounded channel capacity");
    }
    pub fn text(actual: &str, expected: &str) {
        assert_eq!(actual, expected, "complete ordered content");
    }
    pub fn delta(event: Option<StreamEvent>, expected: Option<&str>) {
        match event {
            Some(StreamEvent::TextDelta(text)) => {
                if let Some(expected) = expected {
                    self::text(&text, expected);
                }
            }
            other => panic!("expected text delta: {other:?}"),
        }
    }
    pub fn done(event: Option<StreamEvent>, expected: Option<&str>) {
        match event {
            Some(StreamEvent::Done(response)) => {
                if let Some(expected) = expected {
                    text(response.content.as_deref().unwrap_or_default(), expected);
                }
            }
            other => panic!("expected terminal completion: {other:?}"),
        }
    }
    pub fn closed(event: Option<StreamEvent>) {
        assert!(
            event.is_none(),
            "stream must close after terminal event: {event:?}"
        );
    }
    pub fn not_error(event: &StreamEvent) {
        assert!(
            !matches!(event, StreamEvent::Error(_)),
            "sibling failed: {event:?}"
        );
    }
    pub fn cancelled(actual: bool) {
        assert!(
            actual,
            "cancellation must terminate without successful dispatch"
        );
    }
    pub fn eof(bytes: usize) {
        assert_eq!(bytes, 0, "owned local HTTP transport must shut down");
    }
    pub fn deadline_error(event: Option<StreamEvent>) {
        assert!(
            matches!(event, Some(StreamEvent::Error(_))),
            "deadline must explicitly fail the stream: {event:?}"
        );
    }
    pub fn done_count(previous: bool) {
        assert!(!previous, "exactly one Done");
    }
    pub fn terminal(value: Option<String>) -> String {
        value.expect("stream needs terminal completion")
    }
}
