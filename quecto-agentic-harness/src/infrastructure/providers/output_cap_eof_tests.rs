//! #2210 PR review: a reply whose last output delta has no final newline.
//! A whole-body read parses that last line, so its output is delivered and
//! must count against the cap at end of file: crossing the cap there fails
//! as `OutputCapped`, never `Ok`. A pump hands its handler that last line
//! too (#2249 review), so there it fails as `OutputCapped` as well.
use super::output_cap_provider_tests::{CAP, DELTA, capped_trace, is_cap_error};
use super::stream_idle::tests::{LIVE, bounded, servers};
use super::stream_idle_provider_tests::{Vendor, request, terminations};
use crate::domain::conversation::value_objects::message::Message;
use crate::domain::error::DomainError;
use crate::domain::inference::value_objects::attempt_diagnostics::Termination;
use crate::domain::inference::value_objects::provider::StreamEvent;

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
/// `chat_stream` (its events collected), gated or not. A pump hands the
/// unterminated last line to its handler like any line (#2249 review), so
/// it counts against the cap there too: the attempt fails as
/// `OutputCapped`, the caller's last event the cap error.
#[tokio::test]
async fn a_pump_holds_an_unterminated_last_delta_to_its_cap() {
    for vendor in Vendor::ALL {
        for gated in [false, true] {
            let url = servers::trickling(&[&body_ending_unterminated(vendor)]).await;
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
        }
    }
    for gated in [false, true] {
        let url = servers::trickling(&[&body_ending_unterminated(Vendor::OpenAi)]).await;
        let provider = Vendor::OpenAi.provider(url, gated, LIVE);
        let messages = vec![Message::system("sys"), Message::user("hi")];
        let trace = capped_trace();
        let result = bounded(provider.chat_stream(request(&messages, &trace))).await;
        assert!(
            matches!(&result, Err(DomainError::Provider(m)) if is_cap_error(m)),
            "gated={gated}: {result:?}"
        );
        assert_eq!(
            terminations(&trace),
            [Termination::OutputCapped],
            "gated={gated}"
        );
    }
}
