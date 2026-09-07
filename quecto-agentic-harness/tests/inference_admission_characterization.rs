//! #1679 P0: characterize transport ownership before adding admission.
//! These tests do not implement or claim a shared inference budget.
use std::time::Duration;

use quecto::domain::provider::{ChatRequest, LlmProvider, StreamEvent};
use quecto::infrastructure::providers::openai::OpenAiProvider;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::time::timeout;

struct AbortTask<T>(tokio::task::JoinHandle<T>);
impl<T> Drop for AbortTask<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

const LIMIT: Duration = Duration::from_secs(3);

fn request() -> ChatRequest<'static> {
    ChatRequest {
        messages: &[],
        tools: &[],
        model: "test-model",
        max_tokens: 64,
        temperature: 0.0,
        session_id: Some("characterization"),
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    }
}

#[tokio::test]
async fn incremental_receiver_is_not_transport_completion() {
    // GIVEN real HTTP/SSE whose terminal response cannot arrive until released.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (release, released) = oneshot::channel::<()>();
    let (started, observed) = oneshot::channel::<()>();
    let mut server = AbortTask(tokio::spawn(async move {
        timeout(LIMIT, async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                socket.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
                assert!(request.len() < 16_384);
            }
            let delta = "data: {\"choices\":[{\"delta\":{\"content\":\"first\"}}]}\n\n";
            let done = "data: [DONE]\n\n";
            socket
                .write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{delta}", delta.len() + done.len()).as_bytes())
                .await.unwrap();
            started.send(()).unwrap();
            released.await.unwrap();
            socket.write_all(done.as_bytes()).await.unwrap();
        }).await.expect("fake HTTP server must terminate");
    }));
    let provider = OpenAiProvider::new("fake".into(), Some(format!("http://{address}")));
    // WHEN the receiver is returned, the fake remote response is still open.
    let mut receiver = timeout(LIMIT, provider.chat_stream_incremental(request()))
        .await
        .expect("receiver must return before terminal HTTP response");
    timeout(LIMIT, observed).await.unwrap().unwrap();
    assert!(
        !server.0.is_finished(),
        "terminal response is still withheld"
    );
    match timeout(LIMIT, receiver.recv()).await.unwrap().unwrap() {
        StreamEvent::TextDelta(text) => assert_eq!(text, "first"),
        other => panic!("expected first delta, got {other:?}"),
    }
    assert!(receiver.try_recv().is_err(), "no premature terminal event");
    // THEN terminal delivery happens only after the response is released.
    release.send(()).unwrap();
    match timeout(LIMIT, receiver.recv()).await.unwrap().unwrap() {
        StreamEvent::Done(response) => assert_eq!(response.content.as_deref(), Some("first")),
        other => panic!("expected terminal completion, got {other:?}"),
    }
    assert!(timeout(LIMIT, receiver.recv()).await.unwrap().is_none());
    timeout(LIMIT, &mut server.0).await.unwrap().unwrap();
}

#[tokio::test]
async fn http_rejection_is_a_terminal_error_not_a_successful_stream() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(429).set_body_string("busy"))
        .expect(1)
        .mount(&server)
        .await;
    let provider = OpenAiProvider::new("fake".into(), Some(server.uri()));
    let mut receiver = provider.chat_stream_incremental(request()).await;
    match timeout(LIMIT, receiver.recv()).await.unwrap().unwrap() {
        StreamEvent::Error(error) => assert!(error.contains("HTTP 429"), "{error}"),
        other => panic!("expected rejection, got {other:?}"),
    }
    assert!(timeout(LIMIT, receiver.recv()).await.unwrap().is_none());
}

#[tokio::test]
async fn slow_consumer_preserves_more_deltas_than_the_channel_capacity() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    let deltas: Vec<String> = (0..130).map(|i| format!("{i},")).collect();
    let events: String = deltas
        .iter()
        .map(|text| format!("data: {{\"choices\":[{{\"delta\":{{\"content\":\"{text}\"}}}}]}}\n\n"))
        .collect();
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(format!("{}data: [DONE]\n\n", events)),
        )
        .expect(1)
        .mount(&server)
        .await;
    let provider = OpenAiProvider::new("fake".into(), Some(server.uri()));
    let mut receiver = provider.chat_stream_incremental(request()).await;
    assert_eq!(
        receiver.max_capacity(),
        64,
        "incremental output has a fixed bound"
    );
    // Deliberately do not drain while the producer fills its bounded channel.
    timeout(LIMIT, async {
        while receiver.len() < 64 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("producer must reach the documented channel boundary");
    assert_eq!(
        receiver.capacity(),
        0,
        "producer cannot enqueue beyond the full channel"
    );
    for expected in &deltas {
        match timeout(LIMIT, receiver.recv()).await.unwrap().unwrap() {
            StreamEvent::TextDelta(text) => assert_eq!(&text, expected),
            other => panic!("delta lost or reordered: {other:?}"),
        }
    }
    match timeout(LIMIT, receiver.recv()).await.unwrap().unwrap() {
        StreamEvent::Done(response) => {
            assert_eq!(response.content.as_deref(), Some(deltas.concat().as_str()));
        }
        other => panic!("missing completion: {other:?}"),
    }
    assert!(timeout(LIMIT, receiver.recv()).await.unwrap().is_none());
}

#[tokio::test]
async fn receiver_drop_is_not_a_transport_termination_acknowledgement() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (ready, observed) = oneshot::channel::<()>();
    let (finish, finished) = oneshot::channel::<()>();
    let mut server = AbortTask(tokio::spawn(async move {
        timeout(LIMIT, async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            // Consume the full request so a later read specifically observes EOF.
            let mut headers = Vec::new();
            let mut byte = [0];
            while !headers.ends_with(b"\r\n\r\n") {
                socket.read_exact(&mut byte).await.unwrap();
                headers.push(byte[0]);
                assert!(headers.len() < 16_384);
            }
            let headers = String::from_utf8(headers).unwrap();
            let length: usize = headers.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse().unwrap())
            }).unwrap();
            let mut body = vec![0; length];
            socket.read_exact(&mut body).await.unwrap();
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 14\r\nConnection: close\r\n\r\n").await.unwrap();
            ready.send(()).unwrap();
            finished.await.unwrap();
            // An explicit read timeout characterizes today's detached pump;
            // it is not a remote-inference lifetime assertion.
            assert!(timeout(Duration::from_millis(50), socket.read(&mut byte)).await.is_err(),
                "receiver drop alone currently does not close the HTTP transport");
            socket.write_all(b"data: [DONE]\n\n").await.unwrap();
        }).await.unwrap();
    }));
    let provider = OpenAiProvider::new("fake".into(), Some(format!("http://{address}")));
    let receiver = provider.chat_stream_incremental(request()).await;
    timeout(LIMIT, observed).await.unwrap().unwrap();
    drop(receiver);
    finish.send(()).unwrap();
    timeout(LIMIT, &mut server.0).await.unwrap().unwrap();
}
