//! #2433: a reply that keeps sending events but no output is abandoned at
//! the stream progress bound; one whose model keeps reasoning is not. The
//! reported stall was ~11 recognised Responses events a second for 15
//! minutes with no output. A paused clock stands in for the minutes.
use super::*;
use crate::domain::error::DomainError;
use crate::domain::provider_error::{ProviderErrorClass, classify_provider_error};

/// An event of the Responses wire that carries no output.
const NO_OUTPUT: &str = "data: {\"type\":\"response.in_progress\",\"response\":{}}\n\n";
/// A reasoning summary delta: output, though not visible text.
const REASONING: &str =
    "data: {\"type\":\"response.reasoning_summary_text.delta\",\"delta\":\"thinking\"}\n\n";
/// The progress bound when none is configured.
const PROGRESS: Duration = Duration::from_secs(300);

/// Feed `idle` an event every 100 ms — `REASONING` every `reasoning` when
/// one is given, `NO_OUTPUT` otherwise — for at most `run`: when it ended
/// and its error, or `None` if it ran the whole time.
async fn feed(
    idle: &mut EventIdle,
    reasoning: Option<Duration>,
    run: Duration,
) -> Option<(Duration, Idle)> {
    let started = tokio::time::Instant::now();
    let tick = Duration::from_millis(100);
    let mut next_reasoning = reasoning.map(|every| started + every);
    while started.elapsed() < run {
        let now = tokio::time::Instant::now() + tick;
        let line = match next_reasoning {
            Some(due) if now >= due => {
                next_reasoning = reasoning.map(|every| due + every);
                REASONING
            }
            _ => NO_OUTPUT,
        };
        let arriving = async move {
            tokio::time::sleep(tick).await;
            Ok::<_, ()>(Some(line))
        };
        if let Err(stalled) = idle.next(arriving).await {
            return Some((started.elapsed(), stalled));
        }
    }
    None
}

#[tokio::test(start_paused = true)]
async fn events_without_output_end_at_the_progress_bound_as_a_stall() {
    // An idle bound far past the progress bound: only progress can end it.
    let mut idle = EventIdle::events(StreamIdle::new(Duration::from_secs(1800)));
    let ended = feed(&mut idle, None, Duration::from_secs(600)).await;
    let (at, stalled) = ended.expect("a reply with no output was abandoned");
    assert_eq!(at, PROGRESS, "at the progress bound from the body's start");
    let message = stalled.to_string();
    assert_eq!(
        message,
        "stream progress timeout: the provider sent events but no output for 300 s; \
         the request was abandoned"
    );
    let class = classify_provider_error(&DomainError::Provider(message));
    assert_eq!(
        class,
        ProviderErrorClass::Stalled,
        "retried once, as idle is"
    );
}

#[tokio::test(start_paused = true)]
async fn a_reasoning_delta_every_thirty_seconds_keeps_a_reply_going_for_ten_minutes() {
    let mut idle = EventIdle::events(StreamIdle::default());
    let every = Some(Duration::from_secs(30));
    let ended = feed(&mut idle, every, Duration::from_secs(600)).await;
    assert!(ended.is_none(), "{ended:?}");
}
