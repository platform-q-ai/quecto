//! #2249 review: an Anthropic reply ends whole only at `message_stop`. A
//! body that ends without it is a reply cut short — an error classified as
//! a transport cut (retryable `Network`) — on the streamed read and the
//! whole-body read alike.
use super::*;
use crate::domain::provider::StreamEvent;
use crate::domain::provider_error::{ProviderErrorClass, classify_provider_error};
use crate::infrastructure::providers::sse_common::{SseHandler, SseLineOutcome};

const TEXT: [&str; 2] = [
    "event: content_block_delta",
    r#"data: {"delta":{"type":"text_delta","text":"hi"}}"#,
];

fn is_cut_short(message: &str) -> bool {
    message.contains("ended without completion")
        && classify_provider_error(&DomainError::Provider(message.to_owned()))
            == ProviderErrorClass::Network
}

#[tokio::test]
async fn a_streamed_reply_whose_body_ends_before_message_stop_is_cut_short() {
    for lines in [&TEXT[..], &["event: ping"][..]] {
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let mut handler = AnthropicSseHandler::new(None);
        for line in lines {
            assert!(matches!(
                handler.process_line(line, &tx).await,
                SseLineOutcome::Continue
            ));
        }
        handler.on_eof(&tx).await;
        drop(tx);
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
        }
        assert!(
            matches!(events.last(), Some(StreamEvent::Error(e)) if is_cut_short(e)),
            "{lines:?}: {events:?}"
        );
        assert!(
            !events.iter().any(|e| matches!(e, StreamEvent::Done(_))),
            "{events:?}"
        );
    }
}

#[test]
fn a_whole_body_that_ends_before_message_stop_is_cut_short() {
    let body = TEXT.join("\n");
    let Err(DomainError::Provider(message)) = AnthropicProvider::parse_sse_response(&body, None)
    else {
        panic!("a body without message_stop is a provider error");
    };
    assert!(is_cut_short(&message), "{message}");
    let whole = AnthropicProvider::parse_sse_response(
        &format!("{body}\nevent: message_stop\ndata: {{}}\n"),
        None,
    )
    .unwrap();
    assert_eq!(whole.content.as_deref(), Some("hi"));
}
