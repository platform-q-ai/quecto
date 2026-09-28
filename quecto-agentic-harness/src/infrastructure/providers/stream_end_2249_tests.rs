//! #2249 review round 2: how a stream's body ends decides how the reply
//! ends, on every vendor, streamed or read whole, with admission or
//! without, traced or not.
//!
//! - A last line with no newline is still a line: a terminal event there
//!   ends the reply whole.
//! - `data: [DONE]` with trailing whitespace is `[DONE]`.
//! - A body that ends before its terminal event is cut short: an error the
//!   attempt records as `CutShort`, never `Eof` or `Rejected`; an OpenAI
//!   reply that named its `finish_reason` completed.
//! - A body with no event at all is an empty stream, retried as one.
//! - Usage a cut-short Anthropic reply reported is still counted.
use std::sync::Arc;

use super::stream_idle::tests::{LIVE, bounded, servers};
use super::stream_idle_provider_tests::{Vendor, request, terminations, traced};
use crate::application::providers::ports::ChatRequest;
use crate::domain::attempt_diagnostics::Termination;
use crate::domain::error::DomainError;
use crate::domain::message::Message;
use crate::domain::provider::StreamEvent;
use crate::domain::provider_error::{ProviderErrorClass, classify_provider_error};
use crate::domain::request_observation::RequestTrace;

const DELTA: &str = "whole";

impl Vendor {
    /// A reply's first event and one text delta saying [`DELTA`].
    fn opening(self) -> String {
        match self {
            Vendor::Codex => format!(
                "data: {{\"type\":\"response.created\",\"response\":{{}}}}\n\n\
                 data: {{\"type\":\"response.output_text.delta\",\"delta\":\"{DELTA}\"}}\n\n"
            ),
            Vendor::OpenAi => format!(
                "data: {{\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{DELTA}\"}}}}]}}\n\n"
            ),
            Vendor::Anthropic => format!(
                "event: message_start\ndata: {{\"message\":{{\"usage\":{{\"input_tokens\":1234,\"output_tokens\":1}}}}}}\n\n\
                 event: content_block_start\ndata: {{\"index\":0,\"content_block\":{{\"type\":\"text\",\"text\":\"\"}}}}\n\n\
                 event: content_block_delta\ndata: {{\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\"{DELTA}\"}}}}\n\n"
            ),
        }
    }

    /// The vendor's terminal event, with no newline after it.
    fn terminal(self) -> &'static str {
        match self {
            Vendor::Codex => "data: {\"type\":\"response.completed\",\"response\":{}}",
            Vendor::OpenAi => "data: [DONE]",
            Vendor::Anthropic => "event: message_stop\ndata: {\"type\":\"message_stop\"}",
        }
    }
}

/// How one read of a reply ended.
#[derive(Debug)]
enum Ending {
    Whole(String),
    Failed(String),
}

impl Ending {
    fn from_result(result: Result<crate::domain::message::LlmResponse, DomainError>) -> Self {
        match result {
            Ok(reply) => Ending::Whole(reply.content.unwrap_or_default()),
            Err(DomainError::Provider(message)) => Ending::Failed(message),
            Err(other) => Ending::Failed(other.to_string()),
        }
    }
    fn class(&self) -> Option<ProviderErrorClass> {
        match self {
            Ending::Failed(message) => Some(classify_provider_error(&DomainError::Provider(
                message.clone(),
            ))),
            Ending::Whole(_) => None,
        }
    }
}

/// Every way a reply is read: incremental, and the whole-body reads.
#[derive(Clone, Copy, Debug)]
enum Read {
    Incremental,
    Whole,
}

fn untraced(messages: &[Message]) -> ChatRequest<'_> {
    let mut request = request(messages, &traced());
    request.trace = None;
    request
}

/// Read `body` from `vendor` (gated or not, traced or not) as `read`.
async fn read(
    vendor: Vendor,
    gated: bool,
    trace: Option<&Arc<RequestTrace>>,
    read: Read,
    body: &str,
) -> Ending {
    let url = servers::trickling(&[body]).await;
    let provider = vendor.provider(url, gated, LIVE);
    let messages = vec![Message::system("sys"), Message::user("hi")];
    let request = match trace {
        Some(trace) => request(&messages, trace),
        None => untraced(&messages),
    };
    bounded(async {
        match read {
            Read::Incremental => {
                let mut rx = provider.chat_stream_incremental(request).await;
                let mut last = None;
                while let Some(event) = rx.recv().await {
                    last = Some(event);
                }
                match last {
                    Some(StreamEvent::Done(reply)) => {
                        Ending::Whole(reply.content.unwrap_or_default())
                    }
                    Some(StreamEvent::Error(message)) => Ending::Failed(message),
                    other => Ending::Failed(format!("no terminal event: {other:?}")),
                }
            }
            Read::Whole => Ending::from_result(match vendor {
                Vendor::Codex => provider.chat(request).await,
                _ => provider.chat_stream(request).await,
            }),
        }
    })
    .await
}

/// Every combination a body is read through.
fn every_path() -> Vec<(Vendor, bool, bool, Read)> {
    let mut paths = Vec::new();
    for vendor in Vendor::ALL {
        for gated in [false, true] {
            for traced in [false, true] {
                for read in [Read::Incremental, Read::Whole] {
                    paths.push((vendor, gated, traced, read));
                }
            }
        }
    }
    paths
}

#[tokio::test]
async fn a_terminal_event_on_a_last_line_without_a_newline_ends_the_reply_whole() {
    for (vendor, gated, with_trace, how) in every_path() {
        let body = format!("{}{}", vendor.opening(), vendor.terminal());
        let trace = traced();
        let ending = read(vendor, gated, with_trace.then_some(&trace), how, &body).await;
        let path = format!("{vendor:?} gated={gated} traced={with_trace} {how:?}");
        assert!(
            matches!(&ending, Ending::Whole(text) if text == DELTA),
            "{path}: {ending:?}"
        );
        if with_trace {
            assert_eq!(terminations(&trace), [Termination::Completed], "{path}");
        }
    }
}

#[tokio::test]
async fn done_with_trailing_whitespace_is_done() {
    for vendor in [Vendor::OpenAi, Vendor::Codex] {
        for (gated, how) in [
            (false, Read::Incremental),
            (true, Read::Incremental),
            (false, Read::Whole),
            (true, Read::Whole),
        ] {
            for done in ["data: [DONE] \n\n", "data: [DONE]\t\n\n"] {
                let body = format!("{}{done}", vendor.opening());
                let trace = traced();
                let ending = read(vendor, gated, Some(&trace), how, &body).await;
                let path = format!("{vendor:?} gated={gated} {how:?} {done:?}");
                assert!(
                    matches!(&ending, Ending::Whole(text) if text == DELTA),
                    "{path}: {ending:?}"
                );
                assert_eq!(terminations(&trace), [Termination::Completed], "{path}");
            }
        }
    }
}

#[tokio::test]
async fn a_body_that_ends_before_its_terminal_event_is_cut_short_everywhere() {
    for (vendor, gated, with_trace, how) in every_path() {
        let trace = traced();
        let ending = read(
            vendor,
            gated,
            with_trace.then_some(&trace),
            how,
            &vendor.opening(),
        )
        .await;
        let path = format!("{vendor:?} gated={gated} traced={with_trace} {how:?}");
        assert!(
            matches!(&ending, Ending::Failed(m) if m.contains("ended without completion")),
            "{path}: {ending:?}"
        );
        assert_eq!(ending.class(), Some(ProviderErrorClass::Network), "{path}");
        if with_trace {
            assert_eq!(terminations(&trace), [Termination::CutShort], "{path}");
        }
    }
}

/// An OpenAI reply that named its `finish_reason` completed, though its
/// body then ends without `[DONE]` (some compatible servers never send it).
#[tokio::test]
async fn an_openai_reply_that_named_its_finish_reason_completed() {
    let finish = "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n";
    for gated in [false, true] {
        for how in [Read::Incremental, Read::Whole] {
            let body = format!("{}{finish}", Vendor::OpenAi.opening());
            let trace = traced();
            let ending = read(Vendor::OpenAi, gated, Some(&trace), how, &body).await;
            assert!(
                matches!(&ending, Ending::Whole(text) if text == DELTA),
                "gated={gated} {how:?}: {ending:?}"
            );
            assert_eq!(
                terminations(&trace),
                [Termination::Completed],
                "gated={gated} {how:?}"
            );
        }
    }
}

/// A 200 body with no event at all is an empty stream: retried as one, as
/// before #2249, and recorded as cut short.
#[tokio::test]
async fn a_body_with_no_event_at_all_is_an_empty_stream() {
    for (vendor, gated, with_trace, how) in every_path() {
        for body in ["", ": keepalive\n\n"] {
            let trace = traced();
            let ending = read(vendor, gated, with_trace.then_some(&trace), how, body).await;
            let path = format!("{vendor:?} gated={gated} traced={with_trace} {how:?} {body:?}");
            assert_eq!(
                ending.class(),
                Some(ProviderErrorClass::EmptyStream),
                "{path}: {ending:?}"
            );
            if with_trace {
                assert_eq!(terminations(&trace), [Termination::CutShort], "{path}");
            }
        }
    }
}

/// The input tokens an Anthropic reply reported in `message_start` were
/// spent though the reply was cut short: the request's trace keeps them
/// for accounting, on every read.
#[tokio::test]
async fn a_cut_short_anthropic_reply_keeps_its_reported_usage() {
    for gated in [false, true] {
        for how in [Read::Incremental, Read::Whole] {
            let trace = traced();
            let ending = read(
                Vendor::Anthropic,
                gated,
                Some(&trace),
                how,
                &Vendor::Anthropic.opening(),
            )
            .await;
            assert!(matches!(ending, Ending::Failed(_)), "{ending:?}");
            let usage = trace.unfinished_usage();
            assert_eq!(usage.len(), 1, "gated={gated} {how:?}: {usage:?}");
            assert_eq!(usage[0].prompt_tokens, 1234, "gated={gated} {how:?}");
        }
    }
    // A reply that completed reports its usage with itself, never here.
    let trace = traced();
    let body = format!(
        "{}{}",
        Vendor::Anthropic.opening(),
        Vendor::Anthropic.terminal()
    );
    let ending = read(
        Vendor::Anthropic,
        false,
        Some(&trace),
        Read::Incremental,
        &body,
    )
    .await;
    assert!(matches!(ending, Ending::Whole(_)), "{ending:?}");
    assert!(trace.unfinished_usage().is_empty());
}

/// #2249 review round 3: an `{"error":{}}` chunk names no cause, but a
/// reply cut short by it is never whole — not at a following `[DONE]` nor
/// at a following `finish_reason` — on either OpenAI path: the unknown,
/// retryable 502.
#[tokio::test]
async fn an_empty_error_object_ends_an_openai_reply_as_an_error() {
    let text = "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n";
    let error = "data: {\"error\":{}}\n\n";
    let finish = "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n";
    for after in ["data: [DONE]\n\n", finish] {
        for gated in [false, true] {
            for how in [Read::Incremental, Read::Whole] {
                let body = format!("{text}{error}{after}");
                let ending = read(Vendor::OpenAi, gated, Some(&traced()), how, &body).await;
                let path = format!("gated={gated} {how:?} {after:?}");
                assert!(
                    matches!(&ending, Ending::Failed(m) if m.starts_with("HTTP 502 ")),
                    "{path}: {ending:?}"
                );
                assert_eq!(ending.class(), Some(ProviderErrorClass::Server), "{path}");
            }
        }
    }
}

/// #2249 review round 3: usage an OpenAI or Codex reply reported before
/// it was cut short (an OpenAI usage chunk, a Codex `response.*` event's
/// `usage`) is kept for accounting on every read, as Anthropic's is.
#[tokio::test]
async fn a_cut_short_openai_or_codex_reply_keeps_its_reported_usage() {
    for (vendor, usage) in [
        (
            Vendor::OpenAi,
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":321,\"completion_tokens\":2,\"total_tokens\":323}}\n\n",
        ),
        (
            Vendor::Codex,
            "data: {\"type\":\"response.in_progress\",\"response\":{\"usage\":{\"input_tokens\":321,\"output_tokens\":2}}}\n\n",
        ),
    ] {
        for gated in [false, true] {
            for how in [Read::Incremental, Read::Whole] {
                let trace = traced();
                let body = format!("{}{usage}", vendor.opening());
                let ending = read(vendor, gated, Some(&trace), how, &body).await;
                let path = format!("{vendor:?} gated={gated} {how:?}");
                assert!(matches!(ending, Ending::Failed(_)), "{path}: {ending:?}");
                let spent = trace.unfinished_usage();
                assert_eq!(spent.len(), 1, "{path}: {spent:?}");
                assert_eq!(spent[0].prompt_tokens, 321, "{path}");
                assert_eq!(spent[0].completion_tokens, 2, "{path}");
            }
        }
    }
}
