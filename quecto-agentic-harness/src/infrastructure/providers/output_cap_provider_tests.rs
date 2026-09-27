//! #2210: every provider streaming path — Codex, OpenAI chat and Anthropic;
//! assembled and incremental; with and without an admission gate — follows
//! its attempt live on the request's trace, counts the output it streamed,
//! and stops a reply whose output passes the request's output cap with the
//! output cap error, recorded as `Termination::OutputCapped`.
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::stream_idle::tests::{LIVE, bounded, servers};
use super::stream_idle_provider_tests::{Vendor, request, terminations, traced};
use crate::application::providers::ports::LlmProvider;
use crate::domain::attempt_diagnostics::Termination;
use crate::domain::error::DomainError;
use crate::domain::message::Message;
use crate::domain::provider::StreamEvent;
use crate::domain::provider_error::OUTPUT_CAP_EXCEEDED;
use crate::domain::request_observation::RequestTrace;

/// The cap the runaway tests set: a few dozen deltas.
pub(super) const CAP: u64 = 4096;
/// The output each runaway delta carries.
pub(super) const DELTA: &str = "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijkl";

impl Vendor {
    /// What a runaway reply sends first, and the delta it then repeats.
    pub(super) fn runaway(self) -> (String, String) {
        match self {
            Vendor::Codex => (
                "data: {\"type\":\"response.created\",\"response\":{}}\n\n".into(),
                format!("data: {{\"type\":\"response.output_text.delta\",\"delta\":\"{DELTA}\"}}\n\n"),
            ),
            Vendor::OpenAi => (
                "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"}}]}\n\n".into(),
                format!("data: {{\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{DELTA}\"}}}}]}}\n\n"),
            ),
            Vendor::Anthropic => (
                "event: content_block_start\ndata: {\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n".into(),
                format!("event: content_block_delta\ndata: {{\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\"{DELTA}\"}}}}\n\n"),
            ),
        }
    }

    /// A reply that sends two deltas and then nothing more.
    fn two_deltas(self) -> String {
        let (first, delta) = self.runaway();
        format!("{first}{delta}{delta}")
    }
}

pub(super) fn capped_trace() -> Arc<RequestTrace> {
    let trace = traced();
    trace.set_output_cap(CAP);
    trace
}

pub(super) fn is_cap_error(message: &str) -> bool {
    message.starts_with(OUTPUT_CAP_EXCEEDED) && message.contains(&format!("{CAP} bytes"))
}

/// The attempt stopped just past the cap: within one delta of it.
fn stopped_at_the_cap(trace: &RequestTrace) -> bool {
    let attempts = trace.attempt_diagnostics();
    let output = attempts.first().map_or(0, |attempt| attempt.output_bytes);
    output > CAP && output <= CAP + DELTA.len() as u64
}

#[tokio::test]
async fn a_runaway_incremental_stream_is_stopped_at_its_output_cap() {
    for vendor in Vendor::ALL {
        for gated in [false, true] {
            let (first, delta) = vendor.runaway();
            let url = servers::endless(&first, &delta).await;
            let provider = vendor.provider(url, gated, LIVE);
            let messages = vec![Message::system("sys"), Message::user("hi")];
            let trace = capped_trace();
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
            assert!(
                matches!(&last, Some(StreamEvent::Error(m)) if is_cap_error(m)),
                "{vendor:?} gated={gated}: {last:?}"
            );
            assert_eq!(
                terminations(&trace),
                [Termination::OutputCapped],
                "{vendor:?} gated={gated}"
            );
            assert!(stopped_at_the_cap(&trace), "{vendor:?} gated={gated}");
        }
    }
}

#[tokio::test]
async fn a_runaway_assembled_stream_is_stopped_at_its_output_cap() {
    for vendor in Vendor::ALL {
        for gated in [false, true] {
            let (first, delta) = vendor.runaway();
            let url = servers::endless(&first, &delta).await;
            let provider = vendor.provider(url, gated, LIVE);
            let messages = vec![Message::system("sys"), Message::user("hi")];
            let trace = capped_trace();
            let result = bounded(provider.chat_stream(request(&messages, &trace))).await;
            assert!(
                matches!(&result, Err(DomainError::Provider(m)) if is_cap_error(m)),
                "{vendor:?} gated={gated}: {result:?}"
            );
            assert_eq!(
                terminations(&trace),
                [Termination::OutputCapped],
                "{vendor:?} gated={gated}"
            );
            assert!(stopped_at_the_cap(&trace), "{vendor:?} gated={gated}");
        }
    }
}

/// Codex's `chat` (one-shot agents) reads its SSE body too.
#[tokio::test]
async fn a_runaway_codex_chat_is_stopped_at_its_output_cap() {
    for gated in [false, true] {
        let (first, delta) = Vendor::Codex.runaway();
        let url = servers::endless(&first, &delta).await;
        let provider = Vendor::Codex.provider(url, gated, LIVE);
        let messages = vec![Message::system("sys"), Message::user("hi")];
        let trace = capped_trace();
        let result = bounded(provider.chat(request(&messages, &trace))).await;
        assert!(
            matches!(&result, Err(DomainError::Provider(m)) if is_cap_error(m)),
            "gated={gated}: {result:?}"
        );
        assert_eq!(terminations(&trace), [Termination::OutputCapped]);
    }
}

/// A reply within its cap completes, its output counted on its record.
#[tokio::test]
async fn a_reply_within_its_cap_completes_with_its_output_counted() {
    for vendor in Vendor::ALL {
        for gated in [false, true] {
            let (first, delta) = vendor.runaway();
            let end = match vendor {
                Vendor::Codex => "data: {\"type\":\"response.completed\",\"response\":{}}\n\n",
                Vendor::OpenAi => "data: [DONE]\n\n",
                Vendor::Anthropic => {
                    "event: content_block_stop\ndata: {\"index\":0}\n\nevent: message_stop\ndata: {}\n\n"
                }
            };
            let url = servers::trickling(&[&first, &delta, &delta, end]).await;
            let provider = vendor.provider(url, gated, LIVE);
            let messages = vec![Message::system("sys"), Message::user("hi")];
            let trace = capped_trace();
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
            assert!(
                matches!(last, Some(StreamEvent::Done(_))),
                "{vendor:?} gated={gated}: {last:?}"
            );
            let attempts = trace.attempt_diagnostics();
            assert_eq!(attempts.len(), 1, "{vendor:?} gated={gated}");
            assert_eq!(
                attempts[0].output_bytes,
                2 * DELTA.len() as u64,
                "{vendor:?} gated={gated}"
            );
            assert_ne!(attempts[0].termination, Termination::OutputCapped);
        }
    }
}

/// While a reply streams, its trace shows the attempt in flight: its
/// events, its output and how long since the last event.
#[tokio::test]
async fn an_attempt_in_flight_is_followed_live() {
    for vendor in Vendor::ALL {
        for gated in [false, true] {
            let url = servers::silent_after(&vendor.two_deltas()).await;
            let provider: Arc<dyn LlmProvider> = vendor.provider(url, gated, LIVE);
            let trace = capped_trace();
            let stream = {
                let (provider, trace) = (provider.clone(), trace.clone());
                tokio::spawn(async move {
                    let messages = vec![Message::system("sys"), Message::user("hi")];
                    let mut rx = provider
                        .chat_stream_incremental(request(&messages, &trace))
                        .await;
                    while rx.recv().await.is_some() {}
                })
            };
            let started = Instant::now();
            let progress = loop {
                match trace.attempt_progress(Instant::now()) {
                    Some(progress) if progress.output_bytes == 2 * DELTA.len() as u64 => {
                        break progress;
                    }
                    _ => {
                        assert!(started.elapsed() < LIVE, "{vendor:?} gated={gated}");
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                }
            };
            assert_eq!(progress.number, 1, "{vendor:?} gated={gated}");
            assert!(
                progress.events >= 2,
                "{vendor:?} gated={gated}: {progress:?}"
            );
            assert!(progress.since_last_event_ms.is_some(), "{vendor:?}");
            assert!(progress.first_token_ms.is_some(), "{vendor:?}");
            stream.abort();
        }
    }
}

/// A whole SSE reply read within its cap — Codex `chat`, and every vendor's
/// `chat_stream` — is accepted and ends as its terminal event said, never
/// as refused, its output counted.
#[tokio::test]
async fn an_assembled_reply_within_its_cap_completes_with_its_output_counted() {
    for vendor in Vendor::ALL {
        for gated in [false, true] {
            let (first, delta) = vendor.runaway();
            let end = match vendor {
                Vendor::Codex => "data: {\"type\":\"response.completed\",\"response\":{}}\n\n",
                Vendor::OpenAi => "data: [DONE]\n\n",
                Vendor::Anthropic => {
                    "event: content_block_stop\ndata: {\"index\":0}\n\nevent: message_stop\ndata: {}\n\n"
                }
            };
            let url = servers::trickling(&[&first, &delta, &delta, end]).await;
            let provider = vendor.provider(url, gated, LIVE);
            let messages = vec![Message::system("sys"), Message::user("hi")];
            let trace = capped_trace();
            let result = match vendor {
                Vendor::Codex => bounded(provider.chat(request(&messages, &trace))).await,
                _ => bounded(provider.chat_stream(request(&messages, &trace))).await,
            };
            let expected = DELTA.repeat(2);
            assert_eq!(
                result.map(|reply| reply.content.unwrap_or_default()).ok(),
                Some(expected),
                "{vendor:?} gated={gated}"
            );
            let attempts = trace.attempt_diagnostics();
            assert_eq!(attempts.len(), 1, "{vendor:?} gated={gated}");
            assert_eq!(attempts[0].output_bytes, 2 * DELTA.len() as u64);
            assert!(
                matches!(
                    attempts[0].termination,
                    Termination::Completed | Termination::Eof
                ),
                "{vendor:?} gated={gated}: {:?}",
                attempts[0].termination
            );
        }
    }
}
