//! #2236 / #2249 review: an OpenAI chat-completions reply ends whole only at
//! a terminal signal — `data: [DONE]`, or a chunk whose choice carries a
//! non-empty `finish_reason` (OpenAI-compatible servers such as older
//! llama.cpp and TGI builds end there without `[DONE]`). A body that ends
//! without either is a reply cut short — an error classified as a transport
//! cut (retryable `Network`), never a whole reply — with admission and
//! without alike.
use super::*;
use crate::domain::conversation::value_objects::message::Message;
use crate::domain::error::DomainError;
use crate::domain::inference::services::provider_error::{
    ProviderErrorClass, classify_provider_error,
};
use crate::infrastructure::providers::stream_idle::tests::{LIVE, bounded, servers};
use crate::infrastructure::providers::stream_idle_provider_tests::{Vendor, request, traced};

const TEXT: &str = r#"data: {"choices":[{"index":0,"delta":{"content":"Hi"}}]}"#;
const FINISH: &str = r#"data: {"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#;
const USAGE: &str =
    r#"data: {"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":1,"total_tokens":4}}"#;
const DONE: &str = "data: [DONE]";

/// Feed `lines` to a fresh handler, then end the body unless a line ended
/// the stream (as the pump does); every event sent.
async fn handled(lines: &[&str]) -> Vec<StreamEvent> {
    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    let mut handler = OpenAiSseHandler::new();
    let mut ended = false;
    for line in lines {
        if matches!(handler.process_line(line, &tx).await, SseLineOutcome::Done) {
            ended = true;
            break;
        }
    }
    if !ended {
        handler.on_eof(&tx).await;
    }
    drop(tx);
    let mut events = Vec::new();
    while let Some(event) = rx.recv().await {
        events.push(event);
    }
    events
}

fn is_cut_short(message: &str) -> bool {
    message == OPENAI_CUT_SHORT
        && message.contains("ended without completion")
        && classify_provider_error(&DomainError::Provider(message.to_owned()))
            == ProviderErrorClass::Network
}

fn done_content(events: &[StreamEvent]) -> Option<&str> {
    match events.last() {
        Some(StreamEvent::Done(response)) => response.content.as_deref(),
        other => panic!("expected Done last: {other:?}"),
    }
}

#[test]
fn the_cut_short_error_is_worded_as_a_transport_cut_before_done() {
    assert_eq!(
        OPENAI_CUT_SHORT,
        "OpenAI SSE stream ended without completion: connection closed before [DONE]"
    );
    assert!(is_cut_short(OPENAI_CUT_SHORT));
}

#[tokio::test]
async fn a_streamed_reply_whose_body_ends_before_a_terminal_signal_is_cut_short() {
    let null_reason =
        r#"data: {"choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":null}]}"#;
    let empty_reason =
        r#"data: {"choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":""}]}"#;
    for lines in [
        &[TEXT][..],
        &[TEXT, USAGE][..],
        &[null_reason][..],
        &[empty_reason][..],
        &[r#"data: {"choices":[{"index":0,"delta":{},"finish_reason":42}]}"#][..],
    ] {
        let events = handled(lines).await;
        assert!(
            matches!(events.last(), Some(StreamEvent::Error(e)) if is_cut_short(e)),
            "{lines:?}: {events:?}"
        );
        assert!(
            !events.iter().any(|e| matches!(e, StreamEvent::Done(_))),
            "{lines:?}: {events:?}"
        );
    }
}

#[tokio::test]
async fn done_ends_a_streamed_reply_whole() {
    for lines in [&[TEXT, DONE][..], &[TEXT, FINISH, USAGE, DONE][..]] {
        let events = handled(lines).await;
        assert_eq!(done_content(&events), Some("Hi"), "{lines:?}");
    }
}

/// A server that ends at its `finish_reason` chunk without `[DONE]` sent a
/// whole reply; the usage chunk after it is still read.
#[tokio::test]
async fn a_finish_reason_chunk_ends_a_streamed_reply_whole_without_done() {
    for lines in [&[TEXT, FINISH][..], &[TEXT, FINISH, USAGE][..]] {
        let events = handled(lines).await;
        assert_eq!(done_content(&events), Some("Hi"), "{lines:?}");
        let Some(StreamEvent::Done(response)) = events.last() else {
            unreachable!()
        };
        assert_eq!(
            response.usage.is_some(),
            lines.contains(&USAGE),
            "{lines:?}"
        );
    }
}

/// The finish reason ends the reply only once the body ends: a chunk after
/// it is still handled, never cut off at the finish reason.
#[tokio::test]
async fn a_finish_reason_chunk_does_not_end_the_stream_early() {
    let (tx, _rx) = tokio::sync::mpsc::channel(16);
    let mut handler = OpenAiSseHandler::new();
    for line in [TEXT, FINISH, USAGE] {
        assert!(matches!(
            handler.process_line(line, &tx).await,
            SseLineOutcome::Continue
        ));
    }
}

/// Every streaming path — incremental and assembled, gated by admission or
/// not — ends the same body the same way.
async fn every_path(body: &str) -> Vec<(bool, &'static str, Result<String, String>)> {
    let mut outcomes = Vec::new();
    for gated in [false, true] {
        let messages = vec![Message::system("sys"), Message::user("hi")];
        let url = servers::trickling(&[body]).await;
        let provider = Vendor::OpenAi.provider(url, gated, LIVE);
        let trace = traced();
        let last = bounded(async {
            let mut rx = provider
                .chat_stream_incremental(request(&messages, &trace))
                .await;
            let mut last = None;
            while let Some(event) = rx.recv().await {
                last = Some(event);
            }
            last
        })
        .await;
        let incremental = match last {
            Some(StreamEvent::Done(response)) => Ok(response.content.unwrap_or_default()),
            Some(StreamEvent::Error(message)) => Err(message),
            other => panic!("gated={gated}: {other:?}"),
        };
        outcomes.push((gated, "incremental", incremental));
        let url = servers::trickling(&[body]).await;
        let provider = Vendor::OpenAi.provider(url, gated, LIVE);
        let trace = traced();
        let assembled = bounded(provider.chat_stream(request(&messages, &trace)))
            .await
            .map(|response| response.content.unwrap_or_default())
            .map_err(|error| match error {
                DomainError::Provider(message) => message,
                other => other.to_string(),
            });
        outcomes.push((gated, "assembled", assembled));
    }
    outcomes
}

#[tokio::test]
async fn every_path_ends_a_body_cut_before_a_terminal_signal_as_cut_short() {
    let body = format!("{TEXT}\n\n");
    for (gated, path, outcome) in every_path(&body).await {
        assert!(
            matches!(&outcome, Err(message) if is_cut_short(message)),
            "gated={gated} {path}: {outcome:?}"
        );
    }
}

#[tokio::test]
async fn every_path_ends_a_body_at_a_terminal_signal_whole() {
    for body in [
        format!("{TEXT}\n\n{DONE}\n\n"),
        format!("{TEXT}\n\n{FINISH}\n\n"),
        format!("{TEXT}\n\n{FINISH}\n\n{USAGE}\n\n"),
    ] {
        for (gated, path, outcome) in every_path(&body).await {
            assert_eq!(outcome.as_deref(), Ok("Hi"), "gated={gated} {path}: {body}");
        }
    }
}

/// A later chunk whose choice names no finish reason (a usage chunk some
/// servers send with `finish_reason: null`) never unsays an earlier one.
#[tokio::test]
async fn a_later_chunk_without_a_finish_reason_keeps_the_reply_whole() {
    let usage_choice = r#"data: {"choices":[{"index":0,"delta":{},"finish_reason":null}],"usage":{"prompt_tokens":3,"completion_tokens":1,"total_tokens":4}}"#;
    let events = handled(&[TEXT, FINISH, usage_choice]).await;
    assert_eq!(done_content(&events), Some("Hi"));
}

/// The pump never reads past `[DONE]`, so end of file after it breaks the
/// handler's contract.
#[tokio::test]
#[should_panic(expected = "only without [DONE]")]
async fn end_of_file_after_done_breaks_the_handler_contract() {
    let (tx, _rx) = tokio::sync::mpsc::channel(16);
    let mut handler = OpenAiSseHandler::new();
    assert!(matches!(
        handler.process_line(DONE, &tx).await,
        SseLineOutcome::Done
    ));
    handler.on_eof(&tx).await;
}

/// A body with no event at all is an empty stream (#2249 review round 2):
/// retried as one, as before.
#[tokio::test]
async fn a_body_with_no_event_is_an_empty_stream() {
    for lines in [&[][..], &[": keepalive"][..]] {
        let events = handled(lines).await;
        assert!(
            matches!(events.last(), Some(StreamEvent::Error(e)) if e == crate::domain::inference::services::provider_error::EMPTY_STREAM),
            "{lines:?}: {events:?}"
        );
    }
}
