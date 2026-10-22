//! The Anthropic wire through the shared SSE pump over a real HTTP body: a
//! stream cut before `message_stop` is an error.
use super::AnthropicSseHandler;
use crate::domain::inference::value_objects::provider::StreamEvent;
use crate::infrastructure::providers::transport::sse_common::pump_sse;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn pump_sse_anthropic_wire_stream_cut_before_message_stop_is_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/anthropic"))
        .respond_with(
            ResponseTemplate::new(200).set_body_bytes(
                b"event: content_block_delta\n\
              data: {\"delta\":{\"type\":\"text_delta\",\"text\":\"split\"}}\n"
                    .to_vec(),
            ),
        )
        .mount(&server)
        .await;

    let mut response = reqwest::Client::new()
        .post(format!("{}/anthropic", server.uri()))
        .send()
        .await
        .unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let mut handler = AnthropicSseHandler::new_for_test(None);

    pump_sse(
        &mut response,
        &tx,
        &mut handler,
        Default::default(),
        &super::ANTHROPIC_WIRE,
    )
    .await;

    assert!(matches!(rx.recv().await.unwrap(), StreamEvent::TextDelta(t) if t == "split"));
    // No `message_stop`: the reply was cut short (#2249 review).
    match rx.recv().await.unwrap() {
        StreamEvent::Error(error) => assert!(error.contains("ended without completion"), "{error}"),
        other => panic!("unexpected event: {other:?}"),
    }
}
