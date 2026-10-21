//! The OpenAI SSE pump over a real HTTP body: split lines and `[DONE]`.
use crate::domain::inference::value_objects::provider::StreamEvent;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn pump_sse_openai_wire_stream_handles_split_lines_and_done() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/stream"))
        .respond_with(
            ResponseTemplate::new(200).set_body_bytes(
                b"data: {\"choices\":[{\"delta\":{\"content\":\"he\"}}]}\n\
              data: {\"choices\":[{\"delta\":{\"content\":\"llo\"}}]}\n\
              data: [DONE]\n"
                    .to_vec(),
            ),
        )
        .mount(&server)
        .await;

    let mut response = reqwest::Client::new()
        .post(format!("{}/stream", server.uri()))
        .send()
        .await
        .unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    super::pump_sse_bytes(&mut response, &tx).await;

    assert!(matches!(rx.recv().await.unwrap(), StreamEvent::TextDelta(t) if t == "he"));
    assert!(matches!(rx.recv().await.unwrap(), StreamEvent::TextDelta(t) if t == "llo"));
    match rx.recv().await.unwrap() {
        StreamEvent::Done(done) => assert_eq!(done.content.as_deref(), Some("hello")),
        other => panic!("unexpected event: {other:?}"),
    }
}
