//! #2433: on every streaming path — Codex, OpenAI chat and Anthropic;
//! assembled and incremental; with and without an admission gate — a reply
//! that keeps sending events none of which carries output is abandoned at
//! the progress bound as a stall, recorded as `Termination::NoProgress`.
//! Real sockets, so short bounds in real time.
use std::time::Duration;

use super::stream_idle::StreamIdle;
use super::stream_idle::tests::{LIVE, servers};
use super::stream_idle_provider_tests::{Vendor, assembled, codex_chat, incremental};
use crate::domain::error::DomainError;
use crate::domain::inference::services::provider_error::{
    ProviderErrorClass, classify_provider_error,
};
use crate::domain::inference::value_objects::attempt_diagnostics::Termination;
use crate::domain::inference::value_objects::provider::StreamEvent;

/// The progress bound: well under the idle bound, well over the gaps.
const PROGRESS: Duration = Duration::from_millis(500);
/// The gap between events: fast enough that the events a stall needs
/// ([`super::stream_idle::PROGRESS_EVENTS`]) come within the bound.
const EVERY: Duration = Duration::from_millis(1);

/// The vendor's event that carries no output, sent again and again.
fn no_output(vendor: Vendor) -> &'static str {
    match vendor {
        Vendor::Codex => "data: {\"type\":\"response.in_progress\",\"response\":{}}\n\n",
        Vendor::OpenAi => "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"\"}}]}\n\n",
        Vendor::Anthropic => {
            "event: content_block_delta\ndata: {\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"\"}}\n\n"
        }
    }
}

fn bounds() -> StreamIdle {
    StreamIdle::new(LIVE).with_progress(PROGRESS)
}

fn stalled_message() -> String {
    super::stream_idle::Idle::no_output(PROGRESS).to_string()
}

#[tokio::test]
async fn events_without_output_are_abandoned_at_the_progress_bound() {
    let message = stalled_message();
    assert!(
        message.starts_with("stream progress timeout: "),
        "{message}"
    );
    let class = classify_provider_error(&DomainError::Provider(message.clone()));
    assert_eq!(class, ProviderErrorClass::Stalled);
    for vendor in Vendor::ALL {
        for gated in [false, true] {
            let url = servers::repeating(no_output(vendor), no_output(vendor), EVERY).await;
            let started = std::time::Instant::now();
            let provider = vendor.bounded_provider(url.clone(), gated, bounds());
            let (last, recorded) = incremental(&*provider).await;
            assert!(started.elapsed() >= PROGRESS, "{vendor:?} gated={gated}");
            assert!(
                matches!(&last, Some(StreamEvent::Error(m)) if *m == message),
                "{vendor:?} gated={gated}: {last:?}"
            );
            assert_eq!(recorded, [Termination::NoProgress], "{vendor:?} {gated}");
            let provider = vendor.bounded_provider(url, gated, bounds());
            let (result, recorded) = assembled(&*provider).await;
            assert!(
                matches!(&result, Err(DomainError::Provider(m)) if *m == message),
                "{vendor:?} gated={gated}: {result:?}"
            );
            assert_eq!(recorded, [Termination::NoProgress], "{vendor:?} {gated}");
        }
    }
    for gated in [false, true] {
        let event = no_output(Vendor::Codex);
        let url = servers::repeating(event, event, EVERY).await;
        let provider = Vendor::Codex.bounded_provider(url, gated, bounds());
        let (result, recorded) = codex_chat(&*provider).await;
        assert!(
            matches!(&result, Err(DomainError::Provider(m)) if *m == message),
            "gated={gated}: {result:?}"
        );
        assert_eq!(recorded, [Termination::NoProgress], "gated={gated}");
    }
}

/// Output between the events keeps the reply going past the bound.
#[tokio::test]
async fn output_among_the_events_keeps_a_reply_going() {
    let replies = [
        (
            Vendor::Codex,
            "data: {\"type\":\"response.reasoning_summary_text.delta\",\"delta\":\"hm\"}\n\n",
        ),
        (
            Vendor::OpenAi,
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"reasoning_content\":\"hm\"}}]}\n\n",
        ),
        (
            Vendor::Anthropic,
            "event: content_block_delta\ndata: {\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"hm\"}}\n\n",
        ),
    ];
    for (vendor, output) in replies {
        let url = servers::repeating(no_output(vendor), output, EVERY).await;
        let provider = vendor.bounded_provider(url, false, bounds());
        let messages = vec![
            crate::domain::conversation::value_objects::message::Message::system("sys"),
            crate::domain::conversation::value_objects::message::Message::user("hi"),
        ];
        let trace = super::stream_idle_provider_tests::traced();
        let request = super::stream_idle_provider_tests::request(&messages, &trace);
        let mut rx = provider.chat_stream_incremental(request).await;
        let reading = async {
            while let Some(event) = rx.recv().await {
                if let StreamEvent::Error(error) = event {
                    return Some(error);
                }
            }
            None
        };
        let outlived = tokio::time::timeout(PROGRESS * 4, reading).await;
        assert!(outlived.is_err(), "{vendor:?} ended early: {outlived:?}");
    }
}

/// #2433 review H1: Anthropic's `ping`s while it thinks in hiding are no
/// stall: a reply sending only them is not cut by the progress bound.
#[tokio::test]
async fn pings_alone_are_not_cut_by_the_progress_bound() {
    let ping = "event: ping\ndata: {\"type\":\"ping\"}\n\n";
    for gated in [false, true] {
        let url = servers::repeating(ping, ping, Duration::from_millis(100)).await;
        let provider = Vendor::Anthropic.bounded_provider(url, gated, bounds());
        let messages = vec![
            crate::domain::conversation::value_objects::message::Message::system("sys"),
            crate::domain::conversation::value_objects::message::Message::user("hi"),
        ];
        let trace = super::stream_idle_provider_tests::traced();
        let request = super::stream_idle_provider_tests::request(&messages, &trace);
        let mut rx = provider.chat_stream_incremental(request).await;
        let reading = async {
            while let Some(event) = rx.recv().await {
                if let StreamEvent::Error(error) = event {
                    return Some(error);
                }
            }
            None
        };
        // Under the backstop (three times the progress bound).
        let outlived = tokio::time::timeout(PROGRESS * 5 / 2, reading).await;
        assert!(outlived.is_err(), "gated={gated}: {outlived:?}");
    }
}
