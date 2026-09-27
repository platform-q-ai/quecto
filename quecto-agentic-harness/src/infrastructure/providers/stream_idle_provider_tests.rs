//! #2210: every provider streaming path — Codex, OpenAI chat and Anthropic;
//! assembled and incremental; with and without an admission gate — abandons
//! a reply that goes silent for its idle bound with the idle error, and an
//! observed attempt records it as `Termination::Idle`. Each provider is
//! given a short bound, in real time (a paused clock races loopback I/O).
use std::sync::Arc;

use super::anthropic::AnthropicProvider;
use super::codex::CodexProvider;
use super::openai::OpenAiProvider;
use super::stream_idle::tests::{GAP, LIVE, SILENT, bounded, servers};

/// The total bound for a whole reply: past the idle bound, so a reply the
/// idle bound would end still arrives within it.
const WHOLE: std::time::Duration = std::time::Duration::from_millis(1200);
use crate::application::providers::ports::{ChatRequest, LlmProvider};
use crate::domain::attempt_diagnostics::Termination;
use crate::domain::error::DomainError;
use crate::domain::message::Message;
use crate::domain::provider::StreamEvent;
use crate::domain::request_observation::RequestTrace;

const CODEX_EVENT: &str = "data: {\"type\":\"response.created\",\"response\":{}}\n\n";
const OPENAI_EVENT: &str = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hi\"}}]}\n\n";
const ANTHROPIC_EVENT: &str = "event: ping\ndata: {\"type\":\"ping\"}\n\n";

/// A gate that grants every attempt at once.
#[derive(Debug)]
struct Grant;
#[derive(Debug)]
struct Granted;
impl crate::application::ports::AttemptPermit for Granted {
    fn feedback(&mut self, _: crate::domain::inference_admission::ThrottleFeedback) {}
    fn finish(self: Box<Self>, _: crate::domain::inference_admission::Feedback) {}
}
impl crate::application::ports::AttemptAdmission for Grant {
    fn acquire(&self) -> crate::application::ports::AttemptAcquisition<'_> {
        Box::pin(async {
            Ok(Box::new(Granted) as Box<dyn crate::application::ports::AttemptPermit>)
        })
    }
}
fn single_attempt() -> super::SingleAttemptClient {
    super::SingleAttemptClient::build(reqwest::Client::builder().no_proxy()).unwrap()
}

#[derive(Clone, Copy, Debug)]
enum Vendor {
    Codex,
    OpenAi,
    Anthropic,
}
impl Vendor {
    const ALL: [Vendor; 3] = [Vendor::Codex, Vendor::OpenAi, Vendor::Anthropic];
    /// A whole non-streaming reply saying `ok`.
    fn whole_reply(self) -> &'static str {
        match self {
            Vendor::OpenAi => {
                r#"{"choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}"#
            }
            Vendor::Anthropic => {
                r#"{"content":[{"type":"text","text":"ok"}],"stop_reason":"end_turn"}"#
            }
            Vendor::Codex => unreachable!("Codex replies stream"),
        }
    }
    fn event(self) -> &'static str {
        match self {
            Vendor::Codex => CODEX_EVENT,
            Vendor::OpenAi => OPENAI_EVENT,
            Vendor::Anthropic => ANTHROPIC_EVENT,
        }
    }
    /// The vendor's provider at `url` bounded by `bound`, gated when `gated`.
    fn provider(
        self,
        url: String,
        gated: bool,
        bound: std::time::Duration,
    ) -> Arc<dyn LlmProvider> {
        let client = reqwest::Client::new();
        match (self, gated) {
            (Vendor::Codex, false) => Arc::new(
                CodexProvider::with_client("k".into(), "acct".into(), Some(url), client)
                    .with_stream_idle_limit(bound)
                    .with_reply_total_limit(WHOLE),
            ),
            (Vendor::Codex, true) => Arc::new(
                CodexProvider::with_client("k".into(), "acct".into(), Some(url), client)
                    .with_stream_idle_limit(bound)
                    .with_reply_total_limit(WHOLE)
                    .with_attempt_admission(Arc::new(Grant), single_attempt()),
            ),
            (Vendor::OpenAi, false) => Arc::new(
                OpenAiProvider::with_client("k".into(), Some(url), client)
                    .with_stream_idle_limit(bound)
                    .with_reply_total_limit(WHOLE),
            ),
            (Vendor::OpenAi, true) => Arc::new(
                OpenAiProvider::with_client("k".into(), Some(url), client)
                    .with_stream_idle_limit(bound)
                    .with_reply_total_limit(WHOLE)
                    .with_attempt_admission(Arc::new(Grant), single_attempt()),
            ),
            (Vendor::Anthropic, false) => Arc::new(
                AnthropicProvider::with_client("k".into(), Some(url), client)
                    .with_stream_idle_limit(bound)
                    .with_reply_total_limit(WHOLE),
            ),
            (Vendor::Anthropic, true) => Arc::new(
                AnthropicProvider::with_client("k".into(), Some(url), client)
                    .with_stream_idle_limit(bound)
                    .with_reply_total_limit(WHOLE)
                    .with_attempt_admission(Arc::new(Grant), single_attempt()),
            ),
        }
    }
}

fn request<'a>(messages: &'a [Message], trace: &Arc<RequestTrace>) -> ChatRequest<'a> {
    ChatRequest {
        trace: Some(trace.clone()),
        admission: None,
        model: "gpt-test",
        messages,
        tools: &[],
        max_tokens: 16,
        temperature: 0.0,
        thinking_level: None,
        effort: None,
        tool_choice: None,
        metadata: None,
        session_id: None,
        cancel_flag: None,
    }
}

fn traced() -> Arc<RequestTrace> {
    let trace = Arc::new(RequestTrace::default());
    trace.start();
    trace
}

/// The last event of an incremental stream, and the attempts it recorded.
async fn incremental(provider: &dyn LlmProvider) -> (Option<StreamEvent>, Vec<Termination>) {
    let messages = vec![Message::system("sys"), Message::user("hi")];
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
    (last, terminations(&trace))
}

/// An assembled stream's result, and the attempts it recorded.
async fn assembled(provider: &dyn LlmProvider) -> (Result<String, DomainError>, Vec<Termination>) {
    let messages = vec![Message::system("sys"), Message::user("hi")];
    let trace = traced();
    let result = bounded(provider.chat_stream(request(&messages, &trace))).await;
    let result = result.map(|r| r.content.unwrap_or_default());
    (result, terminations(&trace))
}

/// Codex's `chat` reads its whole SSE body too.
async fn codex_chat(provider: &dyn LlmProvider) -> (Result<String, DomainError>, Vec<Termination>) {
    let messages = vec![Message::system("sys"), Message::user("hi")];
    let trace = traced();
    let result = bounded(provider.chat(request(&messages, &trace))).await;
    let result = result.map(|r| r.content.unwrap_or_default());
    (result, terminations(&trace))
}

fn terminations(trace: &RequestTrace) -> Vec<Termination> {
    trace
        .attempt_diagnostics()
        .iter()
        .map(|a| a.termination)
        .collect()
}

/// The idle error for the [`SILENT`] bound.
fn idle_message() -> String {
    super::stream_idle::tests::idle_message(SILENT)
}

fn is_idle_event(event: &Option<StreamEvent>) -> bool {
    matches!(event, Some(StreamEvent::Error(message)) if *message == idle_message())
}

fn is_idle_error(result: &Result<String, DomainError>) -> bool {
    matches!(result, Err(DomainError::Provider(message)) if *message == idle_message())
}

/// Whether an attempt was recorded as idle: every gated attempt is observed,
/// and an ungated one only where passive observation exists (OpenAI).
fn recorded_idle(vendor: Vendor, gated: bool, recorded: &[Termination]) -> bool {
    match (vendor, gated) {
        (_, true) | (Vendor::OpenAi, false) => recorded == [Termination::Idle],
        (Vendor::Codex | Vendor::Anthropic, false) => recorded.is_empty(),
    }
}

#[tokio::test]
async fn an_incremental_stream_that_goes_silent_mid_body_is_abandoned() {
    for vendor in Vendor::ALL {
        for gated in [false, true] {
            let url = servers::silent_after(vendor.event()).await;
            let (last, recorded) = incremental(&*vendor.provider(url, gated, SILENT)).await;
            assert!(is_idle_event(&last), "{vendor:?} gated={gated}: {last:?}");
            assert!(
                recorded_idle(vendor, gated, &recorded),
                "{vendor:?} gated={gated}: {recorded:?}"
            );
        }
    }
}

#[tokio::test]
async fn an_assembled_stream_that_goes_silent_mid_body_is_abandoned() {
    for vendor in Vendor::ALL {
        for gated in [false, true] {
            let url = servers::silent_after(vendor.event()).await;
            let (result, recorded) = assembled(&*vendor.provider(url, gated, SILENT)).await;
            assert!(
                is_idle_error(&result),
                "{vendor:?} gated={gated}: {result:?}"
            );
            assert!(
                recorded_idle(vendor, gated, &recorded),
                "{vendor:?} gated={gated}: {recorded:?}"
            );
        }
    }
}

#[tokio::test]
async fn codex_chat_that_goes_silent_mid_body_is_abandoned() {
    for gated in [false, true] {
        let url = servers::silent_after(CODEX_EVENT).await;
        let (result, recorded) = codex_chat(&*Vendor::Codex.provider(url, gated, SILENT)).await;
        assert!(is_idle_error(&result), "gated={gated}: {result:?}");
        assert!(
            recorded_idle(Vendor::Codex, gated, &recorded),
            "{recorded:?}"
        );
    }
}

/// The first byte is bounded from the send: a server that never answers.
#[tokio::test]
async fn a_request_no_response_head_answers_is_abandoned() {
    for vendor in Vendor::ALL {
        for gated in [false, true] {
            let url = servers::never_answering().await;
            let started = std::time::Instant::now();
            let provider = vendor.provider(url.clone(), gated, SILENT);
            let (last, recorded) = incremental(&*provider).await;
            assert!(started.elapsed() >= SILENT, "{vendor:?} gated={gated}");
            assert!(is_idle_event(&last), "{vendor:?} gated={gated}: {last:?}");
            assert!(
                recorded_idle(vendor, gated, &recorded),
                "{vendor:?} gated={gated}: {recorded:?}"
            );
            let (result, recorded) = assembled(&*vendor.provider(url, gated, SILENT)).await;
            assert!(
                is_idle_error(&result),
                "{vendor:?} gated={gated}: {result:?}"
            );
            assert!(
                recorded_idle(vendor, gated, &recorded),
                "{vendor:?} gated={gated}: {recorded:?}"
            );
        }
    }
}

/// An error status whose body goes silent still ends: as the HTTP error it
/// is (its class decides retry), marked as abandoned rather than shown
/// empty, and recorded as idle where observed.
#[tokio::test]
async fn an_error_body_that_goes_silent_is_abandoned() {
    let marker = "(error body abandoned: stream idle timeout after 200 ms)";
    for vendor in Vendor::ALL {
        for gated in [false, true] {
            let url = servers::silent_error_body().await;
            let (last, recorded) = incremental(&*vendor.provider(url, gated, SILENT)).await;
            assert!(
                matches!(&last, Some(StreamEvent::Error(m))
                    if m.starts_with("HTTP 500") && m.contains(marker)),
                "{vendor:?} gated={gated}: {last:?}"
            );
            assert!(
                recorded_idle(vendor, gated, &recorded),
                "{vendor:?} gated={gated}: {recorded:?}"
            );
            let url = servers::silent_error_body().await;
            let (result, recorded) = assembled(&*vendor.provider(url, gated, SILENT)).await;
            assert!(
                matches!(&result, Err(DomainError::Provider(m))
                    if m.starts_with("HTTP 500") && m.contains(marker)),
                "{vendor:?} gated={gated}: {result:?}"
            );
            assert!(
                recorded_idle(vendor, gated, &recorded),
                "{vendor:?} gated={gated}: {recorded:?}"
            );
        }
    }
}

/// A reply that keeps sending completes on every path: each gap is well
/// within the bound, however long the whole reply runs.
async fn completes_while_sending(vendor: Vendor, events: &[&str]) {
    for gated in [false, true] {
        let url = servers::trickling(events).await;
        let started = std::time::Instant::now();
        let (result, _) = assembled(&*vendor.provider(url.clone(), gated, LIVE)).await;
        assert_eq!(result.unwrap(), "ab", "{vendor:?} gated={gated}");
        assert!(started.elapsed() >= GAP * events.len() as u32, "{vendor:?}");
        let (last, _) = incremental(&*vendor.provider(url, gated, LIVE)).await;
        assert!(
            matches!(last, Some(StreamEvent::Done(_))),
            "{vendor:?} gated={gated}: {last:?}"
        );
    }
}

#[tokio::test]
async fn a_codex_reply_that_keeps_sending_completes() {
    let events = [
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"a\"}\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"b\"}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{}}\n\n",
    ];
    completes_while_sending(Vendor::Codex, &events).await;
}

#[tokio::test]
async fn an_openai_reply_that_keeps_sending_completes() {
    let events = [
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"a\"}}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"b\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    ];
    completes_while_sending(Vendor::OpenAi, &events).await;
}

#[tokio::test]
async fn an_anthropic_reply_that_keeps_sending_completes() {
    let events = [
        "event: content_block_start\ndata: {\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\ndata: {\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"a\"}}\n\n",
        "event: content_block_delta\ndata: {\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"b\"}}\n\n",
        "event: content_block_stop\ndata: {\"index\":0}\n\n",
        "event: message_stop\ndata: {}\n\n",
    ];
    completes_while_sending(Vendor::Anthropic, &events).await;
}

/// A whole JSON reply sends nothing until it is complete, so it is bounded
/// in total, never by the idle bound: one that is silent for longer than the
/// idle bound but arrives within the total one is read.
#[tokio::test]
async fn a_whole_reply_is_bounded_in_total_not_by_silence() {
    for vendor in [Vendor::OpenAi, Vendor::Anthropic] {
        let url = servers::answering_late(SILENT * 3, vendor.whole_reply()).await;
        for gated in [false, true] {
            let provider = vendor.provider(url.clone(), gated, SILENT);
            let messages = vec![Message::system("sys"), Message::user("hi")];
            let response = bounded(provider.chat(request(&messages, &traced()))).await;
            assert_eq!(
                response.unwrap().content.as_deref(),
                Some("ok"),
                "{vendor:?} gated={gated}"
            );
        }
    }
}

/// #2210 review: a whole reply that never arrives is abandoned at the total
/// bound, as a timeout, recorded as `Termination::TimedOut` where observed.
#[tokio::test]
async fn a_whole_reply_that_never_arrives_is_abandoned_at_the_total_limit() {
    let expected = super::stream_idle::tests::timed_out_message(WHOLE);
    for vendor in [Vendor::OpenAi, Vendor::Anthropic] {
        for gated in [false, true] {
            let url = servers::never_answering().await;
            let provider = vendor.provider(url, gated, SILENT);
            let messages = vec![Message::system("sys"), Message::user("hi")];
            let trace = traced();
            let started = std::time::Instant::now();
            let result = bounded(provider.chat(request(&messages, &trace))).await;
            assert!(started.elapsed() >= WHOLE, "{vendor:?} gated={gated}");
            assert!(
                matches!(&result, Err(DomainError::Provider(m)) if *m == expected),
                "{vendor:?} gated={gated}: {result:?}"
            );
            let recorded = terminations(&trace);
            let observed = gated || matches!(vendor, Vendor::OpenAi);
            match observed {
                true => assert_eq!(recorded, [Termination::TimedOut], "{vendor:?} {gated}"),
                false => assert!(recorded.is_empty(), "{vendor:?} {gated}: {recorded:?}"),
            }
        }
    }
}
