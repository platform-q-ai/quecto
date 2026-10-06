//! #2210: a silent provider stream is abandoned after its idle bound; a
//! slow one that keeps sending is not. Tests with sockets use a short bound
//! in real time (a paused clock races loopback I/O); the rest pause it.
use super::*;
use crate::domain::error::DomainError;
use crate::domain::inference::services::provider_error::{
    ProviderErrorClass, classify_provider_error,
};
use crate::domain::inference::value_objects::provider::StreamEvent;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The bound for a stream that goes silent: short, as only its expiry is
/// awaited.
pub(crate) const SILENT: Duration = Duration::from_millis(200);
/// The bound for a stream that keeps sending: far longer than its gaps, so
/// a loaded machine never mistakes a gap for silence.
pub(crate) const LIVE: Duration = Duration::from_millis(1500);
/// A gap between the events of a stream that keeps sending.
pub(crate) const GAP: Duration = Duration::from_millis(100);
/// A gap between the keep-alives of a reply held open (#2433): far shorter
/// than [`SILENT`], so only an idle bound that keep-alives do not restart
/// ever ends it.
pub(crate) const KEEP_ALIVE: Duration = Duration::from_millis(20);

/// Local HTTP servers that misbehave in time, for the idle-bound tests.
pub(crate) mod servers {
    use super::*;

    /// One chunk of a chunked HTTP/1.1 body.
    fn chunk(data: &str) -> String {
        format!("{:x}\r\n{data}\r\n", data.len())
    }

    const HEAD: &str = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
                        transfer-encoding: chunked\r\n\r\n";

    /// Serve every connection: after reading the request, wait `late`, write
    /// `head`, then each step waits its delay and writes its raw bytes; `None`
    /// for the head writes nothing at all. When `silent` the connection is
    /// then held open, sending nothing, for ever; otherwise it closes.
    async fn serve(
        late: Duration,
        head: Option<String>,
        steps: Vec<(Duration, String)>,
        silent: bool,
    ) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let (head, steps) = (head.clone(), steps.clone());
                tokio::spawn(async move {
                    let mut request = vec![0u8; 64 * 1024];
                    let _ = socket.read(&mut request).await;
                    tokio::time::sleep(late).await;
                    if let Some(head) = head {
                        socket.write_all(head.as_bytes()).await.unwrap();
                        for (delay, data) in steps {
                            tokio::time::sleep(delay).await;
                            socket.write_all(data.as_bytes()).await.unwrap();
                        }
                    }
                    if silent {
                        std::future::pending::<()>().await;
                    }
                });
            }
        });
        format!("http://{address}")
    }

    /// Answers with a head and `sent`, then sends nothing more.
    pub(crate) async fn silent_after(sent: &str) -> String {
        let steps = vec![(Duration::ZERO, chunk(sent))];
        serve(Duration::ZERO, Some(HEAD.into()), steps, true).await
    }

    /// Reads the request and never answers.
    pub(crate) async fn never_answering() -> String {
        serve(Duration::ZERO, None, Vec::new(), true).await
    }

    /// Sends each event [`GAP`] after the last, then ends the body.
    pub(crate) async fn trickling(events: &[&str]) -> String {
        let mut steps: Vec<_> = events.iter().map(|e| (GAP, chunk(e))).collect();
        steps.push((Duration::ZERO, "0\r\n\r\n".into()));
        serve(Duration::ZERO, Some(HEAD.into()), steps, false).await
    }

    /// Answers with a head and the event `first`, then sends only
    /// keep-alives — an SSE comment, then a blank line, each [`KEEP_ALIVE`]
    /// after the last — for as long as the client reads: a reply held open
    /// that makes no progress (#2433).
    pub(crate) async fn keeping_alive(first: &str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let head = format!("{HEAD}{}", chunk(first));
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let head = head.clone();
                tokio::spawn(async move {
                    let mut request = vec![0u8; 64 * 1024];
                    let _ = socket.read(&mut request).await;
                    let mut sent = socket.write_all(head.as_bytes()).await;
                    for keep_alive in [": keepalive\n", "\n"].iter().cycle() {
                        tokio::time::sleep(KEEP_ALIVE).await;
                        if sent.is_err() {
                            break;
                        }
                        sent = socket.write_all(chunk(keep_alive).as_bytes()).await;
                    }
                });
            }
        });
        format!("http://{address}")
    }

    /// Answers with a head and `first`, then sends `repeated` every
    /// `every` for as long as the client reads: a reply that keeps sending
    /// the same event (#2433).
    pub(crate) async fn repeating(first: &str, repeated: &str, every: Duration) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (head, repeated) = (format!("{HEAD}{}", chunk(first)), chunk(repeated));
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let (head, repeated) = (head.clone(), repeated.clone());
                tokio::spawn(async move {
                    let mut request = vec![0u8; 64 * 1024];
                    let _ = socket.read(&mut request).await;
                    let mut sent = socket.write_all(head.as_bytes()).await;
                    while sent.is_ok() {
                        tokio::time::sleep(every).await;
                        sent = socket.write_all(repeated.as_bytes()).await;
                    }
                });
            }
        });
        format!("http://{address}")
    }

    /// Sends `first`, then `repeated` again and again as fast as it is read,
    /// until the client goes away: a runaway reply (#2210).
    pub(crate) async fn endless(first: &str, repeated: &str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (first, repeated) = (chunk(first), chunk(repeated));
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let (first, repeated) = (first.clone(), repeated.clone());
                tokio::spawn(async move {
                    let mut request = vec![0u8; 64 * 1024];
                    let _ = socket.read(&mut request).await;
                    let head = format!("{HEAD}{first}");
                    let mut sent = socket.write_all(head.as_bytes()).await;
                    while sent.is_ok() {
                        sent = socket.write_all(repeated.as_bytes()).await;
                    }
                });
            }
        });
        format!("http://{address}")
    }

    /// Answers 500 with part of the body it promises, then goes silent.
    pub(crate) async fn silent_error_body() -> String {
        let head = "HTTP/1.1 500 Internal Server Error\r\ncontent-length: 100\r\n\r\n";
        let steps = vec![(Duration::ZERO, "{\"err".to_owned())];
        serve(Duration::ZERO, Some(head.into()), steps, true).await
    }

    /// Answers a whole JSON `body` only after `late`.
    pub(crate) async fn answering_late(late: Duration, body: &str) -> String {
        let head = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        );
        serve(late, Some(head), Vec::new(), false).await
    }
}

/// The timeout error for a `total` bound.
pub(crate) fn timed_out_message(total: Duration) -> String {
    TimedOut(total).to_string()
}

/// The idle error for a `limit` bound.
pub(crate) fn idle_message(limit: Duration) -> String {
    Idle::nothing(limit).to_string()
}

/// The idle error of an SSE body for a `limit` bound (#2433 review).
pub(crate) fn no_event_message(limit: Duration) -> String {
    Idle::no_event(limit).to_string()
}

/// Await `step`, failing the test when nothing bounded it: an unbounded
/// silent stream would otherwise wait for ever.
pub(crate) async fn bounded<F: std::future::Future>(step: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(20), step)
        .await
        .expect("the silent stream was bounded")
}

#[test]
fn an_expiry_reads_as_a_stall_retried_once() {
    let message = Idle::nothing(STREAM_IDLE_LIMIT).to_string();
    assert_eq!(
        message,
        "stream idle timeout: the provider sent nothing for 300 s; the request was abandoned"
    );
    let late = TimedOut(REPLY_TOTAL_LIMIT).to_string();
    assert_eq!(
        late,
        "reply timeout: the provider sent no whole reply within 1200 s; the request was abandoned"
    );
    for message in [message, late] {
        let class = classify_provider_error(&DomainError::Provider(message));
        assert_eq!(class, ProviderErrorClass::Stalled);
        assert!(class.is_retryable());
        assert_eq!(class.max_failures(), Some(2));
    }
    let short = Idle::no_event(SILENT);
    assert_eq!(
        short.to_string(),
        "stream idle timeout: the provider sent no event for 200 ms; the request was abandoned"
    );
    assert_eq!(BodyError::Idle(short).to_string(), short.to_string());
}

#[test]
fn a_provider_bound_is_the_limit_unless_chosen() {
    assert_eq!(StreamIdle::default().limit(), STREAM_IDLE_LIMIT);
    assert_eq!(StreamIdle::default().total(), REPLY_TOTAL_LIMIT);
    let chosen = StreamIdle::new(SILENT);
    assert_eq!(
        (chosen.limit(), chosen.total()),
        (SILENT, REPLY_TOTAL_LIMIT)
    );
    let chosen = chosen.with_total(LIVE);
    assert_eq!((chosen.limit(), chosen.total()), (SILENT, LIVE));
    assert!(std::panic::catch_unwind(|| StreamIdle::new(Duration::ZERO)).is_err());
    let zero_total = || StreamIdle::default().with_total(Duration::ZERO);
    assert!(std::panic::catch_unwind(zero_total).is_err());
}

#[test]
fn an_abandoned_error_body_is_marked_and_a_failed_one_is_empty() {
    let marker = "(error body abandoned: stream idle timeout after 300 s)";
    assert_eq!(Idle::nothing(STREAM_IDLE_LIMIT).body_marker(), marker);
    assert_eq!(
        error_text(Err(BodyError::Idle(Idle::nothing(STREAM_IDLE_LIMIT)))),
        marker
    );
    assert_eq!(error_text(Ok("{\"error\":1}".into())), "{\"error\":1}");
}

/// A whole exchange is bounded in total, however it spends the time.
#[tokio::test(start_paused = true)]
async fn a_whole_exchange_over_the_total_bound_times_out() {
    let bound = StreamIdle::default();
    let started = tokio::time::Instant::now();
    let never = bound.whole(std::future::pending::<()>()).await;
    assert_eq!(never, Err(TimedOut(REPLY_TOTAL_LIMIT)));
    assert_eq!(started.elapsed(), REPLY_TOTAL_LIMIT);
    let slow = async {
        tokio::time::sleep(REPLY_TOTAL_LIMIT - Duration::from_millis(1)).await;
        7
    };
    assert_eq!(bound.whole(slow).await, Ok(7));
}

#[tokio::test(start_paused = true)]
async fn a_step_that_waits_the_whole_bound_is_idle() {
    let idle = StreamIdle::default();
    let started = tokio::time::Instant::now();
    let silent = idle.within(std::future::pending::<()>()).await;
    assert_eq!(silent, Err(Idle::nothing(STREAM_IDLE_LIMIT)));
    assert_eq!(started.elapsed(), STREAM_IDLE_LIMIT);
    assert_eq!(idle.within(async { 7 }).await, Ok(7));
    let slow = async {
        tokio::time::sleep(STREAM_IDLE_LIMIT - Duration::from_millis(1)).await;
        7
    };
    assert_eq!(idle.within(slow).await, Ok(7));
}

#[tokio::test]
async fn a_whole_body_that_goes_silent_is_idle() {
    let url = servers::silent_after("data: partial\n").await;
    let response = reqwest::Client::new().get(url).send().await.unwrap();
    let read = bounded(StreamIdle::new(SILENT).text(response)).await;
    assert!(
        matches!(read, Err(BodyError::Idle(idle)) if idle == Idle::nothing(SILENT)),
        "{read:?}"
    );
}

#[tokio::test]
async fn a_whole_body_that_keeps_sending_is_read_past_the_bound() {
    let events = ["data: 1\n"; 16];
    let bound = LIVE;
    let url = servers::trickling(&events).await;
    let started = std::time::Instant::now();
    let response = reqwest::Client::new().get(url).send().await.unwrap();
    let read = bounded(StreamIdle::new(bound).text(response)).await;
    assert_eq!(read.unwrap(), events.concat());
    assert!(started.elapsed() > bound, "the total is not bounded");
}

/// A handler recording lines; its end of file sends a marker.
#[derive(Default)]
struct Lines(Vec<String>);
impl super::super::sse_common::SseHandler for Lines {
    async fn process_line(
        &mut self,
        line: &str,
        _: &tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> super::super::sse_common::SseLineOutcome {
        self.0.push(line.to_owned());
        super::super::sse_common::SseLineOutcome::Continue
    }
    async fn on_eof(&mut self, tx: &tokio::sync::mpsc::Sender<StreamEvent>) {
        let _ = tx.send(StreamEvent::TextDelta("eof".into())).await;
    }
}

#[tokio::test]
async fn the_shared_pump_ends_a_silent_stream_with_the_idle_error() {
    let url = servers::silent_after("data: one\n").await;
    let mut response = reqwest::Client::new().get(url).send().await.unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let mut handler = Lines::default();
    let idle = StreamIdle::new(SILENT);
    bounded(super::super::sse_common::pump_sse(
        &mut response,
        &tx,
        &mut handler,
        idle,
    ))
    .await;
    drop(tx);
    assert_eq!(handler.0, vec!["data: one".to_owned()]);
    let expected = Idle::no_event(SILENT).to_string();
    assert!(
        matches!(rx.recv().await, Some(StreamEvent::Error(m)) if m == expected),
        "the stream fails with the idle error"
    );
    assert!(rx.recv().await.is_none(), "and nothing after it");
}

#[tokio::test]
async fn the_shared_pump_reads_a_slow_stream_past_the_bound() {
    let events = ["data: one\n"; 16];
    let url = servers::trickling(&events).await;
    let bound = LIVE;
    let started = std::time::Instant::now();
    let mut response = reqwest::Client::new().get(url).send().await.unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let mut handler = Lines::default();
    let idle = StreamIdle::new(bound);
    super::super::sse_common::pump_sse(&mut response, &tx, &mut handler, idle).await;
    assert_eq!(handler.0, ["data: one"; 16]);
    assert!(matches!(rx.recv().await, Some(StreamEvent::TextDelta(e)) if e == "eof"));
    assert!(started.elapsed() > bound, "the total is not bounded");
}

#[tokio::test]
async fn a_send_no_response_head_answers_is_idle() {
    let url = servers::never_answering().await;
    let started = std::time::Instant::now();
    let idle = StreamIdle::new(SILENT);
    let sent = bounded(idle.within(reqwest::Client::new().get(url).send())).await;
    assert_eq!(sent.unwrap_err(), Idle::nothing(SILENT));
    assert!(started.elapsed() >= SILENT);
}

/// Read `chunks` through `idle`, each arriving [`GAP`] after the last (a
/// paused clock): whether every read came within the bound.
async fn within_after(idle: &mut EventIdle, chunks: &[&str]) -> bool {
    for chunk in chunks {
        let arriving = async {
            tokio::time::sleep(GAP).await;
            Ok::<_, ()>(Some(*chunk))
        };
        if idle.next(arriving).await.is_err() {
            return false;
        }
    }
    true
}

/// #2433: an SSE body's bound restarts at the bytes of an event — a `data:`
/// line, however the transport splits it — and at nothing else.
#[tokio::test(start_paused = true)]
async fn an_sse_bound_restarts_at_events_only() {
    let bound = StreamIdle::new(GAP * 3);
    // Keep-alives alone: comments, blank lines, other fields, empty chunks.
    let keep_alives = [": keepalive\n", "\n", "", "event: ping\n", "\r\n", ": x"];
    assert!(!within_after(&mut EventIdle::events(bound), &keep_alives).await);
    // Each event restarts it, split across reads or not.
    let events = [
        "data: 1\n\n",
        "da",
        "ta: 2",
        "\n",
        // Counted, or the keep-alives after it outlast the bound from `2`.
        "data:3\n",
        ": keepalive\n",
        ": keepalive\n",
    ];
    assert!(within_after(&mut EventIdle::events(bound), &events).await);
    // `data` must open the line: inside a comment it is no event.
    let inside = [": data: 1\n", " data: 2\n", "\n", "x"];
    assert!(!within_after(&mut EventIdle::events(bound), &inside).await);
    // A long event line still arriving is progress.
    let long = ["data: {", "\"a\":", "1", "}", "\n"];
    assert!(within_after(&mut EventIdle::events(bound), &long).await);
    // Any other body: every read restarts it.
    assert!(within_after(&mut EventIdle::reads(bound), &keep_alives).await);
}

/// The whole bound passes from the last event however the reads go on.
#[tokio::test(start_paused = true)]
async fn an_sse_bound_runs_from_the_last_event() {
    let mut idle = EventIdle::events(StreamIdle::new(GAP * 3));
    let started = tokio::time::Instant::now();
    assert!(within_after(&mut idle, &["data: 1\n", ": a\n", ": b\n"]).await);
    let pending = idle.next(std::future::pending::<Result<Option<&str>, ()>>());
    assert_eq!(pending.await, Err(Idle::no_event(GAP * 3)));
    assert_eq!(
        started.elapsed(),
        GAP + GAP * 3,
        "from the event, not the keep-alives"
    );
}

#[tokio::test]
async fn a_whole_sse_body_that_only_keeps_alive_is_idle() {
    let url = servers::keeping_alive("data: one\n\n").await;
    let response = reqwest::Client::new().get(url).send().await.unwrap();
    let read = bounded(StreamIdle::new(SILENT).sse_text(response)).await;
    assert!(
        matches!(read, Err(BodyError::Idle(idle)) if idle == Idle::no_event(SILENT)),
        "{read:?}"
    );
}

#[tokio::test]
async fn a_whole_sse_body_that_keeps_sending_events_is_read_past_the_bound() {
    let events = ["data: 1\n"; 16];
    let url = servers::trickling(&events).await;
    let started = std::time::Instant::now();
    let response = reqwest::Client::new().get(url).send().await.unwrap();
    let read = bounded(StreamIdle::new(LIVE).sse_text(response)).await;
    assert_eq!(read.unwrap(), events.concat());
    assert!(started.elapsed() > LIVE, "the total is not bounded");
}

/// #2433 review: an idle error says what was missing — nothing at all
/// before a response head or in an error body, no event in an SSE body.
#[tokio::test(start_paused = true)]
async fn the_idle_errors_say_what_was_missing() {
    let nothing =
        "stream idle timeout: the provider sent nothing for 200 ms; the request was abandoned";
    let no_event =
        "stream idle timeout: the provider sent no event for 200 ms; the request was abandoned";
    let bound = StreamIdle::new(SILENT);
    let never = || std::future::pending::<Result<Option<&str>, ()>>();
    let send = bound.within(never()).await.unwrap_err();
    assert_eq!(send.to_string(), nothing, "the send");
    let sse = EventIdle::events(bound).next(never()).await.unwrap_err();
    assert_eq!(sse.to_string(), no_event, "an SSE body");
    let body = EventIdle::reads(bound).next(never()).await.unwrap_err();
    assert_eq!(body.to_string(), nothing, "an error body");
}

/// #2433 review: the idle bound reads events by the one rule the parsers
/// and the event count use, [`super::super::sse_common::event_data`].
#[tokio::test(start_paused = true)]
async fn the_idle_bound_and_the_parsers_agree_on_events() {
    let lines = [
        "data: {}",
        "data:{}",
        "data:",
        "data: [DONE]",
        " data: x",
        "\tdata: x",
        ": data: x",
        "event: ping",
        "id: 1",
        "",
        "dat",
        "datum: 1",
        "DATA: 1",
        "data :x",
    ];
    for line in lines {
        let mut idle = EventIdle::events(StreamIdle::new(GAP));
        tokio::time::sleep(GAP / 2).await;
        let read = idle
            .next(async { Ok::<_, ()>(Some(format!("{line}\n"))) })
            .await;
        assert!(read.is_ok(), "{line:?}");
        tokio::time::sleep(GAP / 2 + Duration::from_millis(1)).await;
        let event = super::super::sse_common::event_data(line).is_some();
        let pending = idle.next(std::future::pending::<Result<Option<String>, ()>>());
        let restarted = tokio::time::timeout(Duration::ZERO, pending).await.is_err();
        assert_eq!(restarted, event, "{line:?}");
    }
}
