//! #2210 PR review: a reply whose last output delta has no final newline.
//! A whole-body read parses that last line, so its output is delivered and
//! must count against the cap at end of file: crossing the cap there fails
//! as `OutputCapped`, never `Ok`. A pump hands its handler only complete
//! lines, so there the unterminated delta is never delivered, and what is
//! delivered stays within the cap.
use super::output_cap_provider_tests::{CAP, DELTA, capped_trace, is_cap_error};
use super::stream_idle::tests::{LIVE, bounded, servers};
use super::stream_idle_provider_tests::{Vendor, request, terminations};
use crate::domain::attempt_diagnostics::Termination;
use crate::domain::error::DomainError;
use crate::domain::message::Message;
use crate::domain::provider::StreamEvent;

/// A body of complete deltas exactly filling the cap, then one more delta
/// with no final newline: only the unterminated one crosses the cap.
fn body_ending_unterminated(vendor: Vendor) -> String {
    let (first, delta) = vendor.runaway();
    let within = (CAP / DELTA.len() as u64) as usize;
    let last = delta.trim_end_matches('\n');
    format!("{first}{}{last}", delta.repeat(within))
}

/// The whole-body reads: Codex `chat` and Anthropic `chat_stream`, gated
/// (admission-owned `assembled`) and ungated (passive `read_sse`).
#[tokio::test]
async fn a_whole_read_ending_in_an_unterminated_delta_past_its_cap_is_output_capped() {
    for (vendor, gated) in [
        (Vendor::Codex, true),
        (Vendor::Anthropic, true),
        (Vendor::Codex, false),
        (Vendor::Anthropic, false),
    ] {
        let url = servers::trickling(&[&body_ending_unterminated(vendor)]).await;
        let provider = vendor.provider(url, gated, LIVE);
        let messages = vec![Message::system("sys"), Message::user("hi")];
        let trace = capped_trace();
        let result = match vendor {
            Vendor::Codex => bounded(provider.chat(request(&messages, &trace))).await,
            _ => bounded(provider.chat_stream(request(&messages, &trace))).await,
        };
        assert!(
            matches!(&result, Err(DomainError::Provider(m)) if is_cap_error(m)),
            "{vendor:?} gated={gated}: {result:?}"
        );
        assert_eq!(
            terminations(&trace),
            [Termination::OutputCapped],
            "{vendor:?} gated={gated}"
        );
        assert_eq!(
            trace.attempt_diagnostics()[0].output_bytes,
            CAP + DELTA.len() as u64,
            "{vendor:?} gated={gated}"
        );
    }
}

/// The pumps: every vendor's incremental stream, gated or not, and OpenAI
/// `chat_stream` (its events collected), gated or not. The unterminated delta never reaches the
/// caller, so nothing past the cap does.
#[tokio::test]
async fn a_pump_never_delivers_an_unterminated_delta_past_its_cap() {
    for vendor in Vendor::ALL {
        for gated in [false, true] {
            let url = servers::trickling(&[&body_ending_unterminated(vendor)]).await;
            let provider = vendor.provider(url, gated, LIVE);
            let messages = vec![Message::system("sys"), Message::user("hi")];
            let trace = capped_trace();
            let delivered = bounded(async {
                let mut rx = provider
                    .chat_stream_incremental(request(&messages, &trace))
                    .await;
                let mut delivered = 0u64;
                while let Some(event) = rx.recv().await {
                    if let StreamEvent::TextDelta(text) = event {
                        delivered += text.len() as u64;
                    }
                }
                delivered
            })
            .await;
            assert!(delivered <= CAP, "{vendor:?} gated={gated}: {delivered}");
            let attempts = trace.attempt_diagnostics();
            assert!(attempts[0].output_bytes <= CAP, "{vendor:?} gated={gated}");
            assert_ne!(attempts[0].termination, Termination::OutputCapped);
        }
    }
    for gated in [false, true] {
        let url = servers::trickling(&[&body_ending_unterminated(Vendor::OpenAi)]).await;
        let provider = Vendor::OpenAi.provider(url, gated, LIVE);
        let messages = vec![Message::system("sys"), Message::user("hi")];
        let trace = capped_trace();
        let result = bounded(provider.chat_stream(request(&messages, &trace))).await;
        let delivered = result.map_or(0, |reply| reply.content.unwrap_or_default().len() as u64);
        assert!(delivered <= CAP, "gated={gated}: {delivered}");
        assert!(
            trace.attempt_diagnostics()[0].output_bytes <= CAP,
            "gated={gated}"
        );
    }
}
