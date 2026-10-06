//! #2433: a reply that keeps sending events but no output is abandoned at
//! the stream progress bound; one whose model keeps reasoning is not. The
//! reported stall was ~11 recognised Responses events a second for 15
//! minutes with no output. A paused clock stands in for the minutes.
use super::*;
use crate::domain::error::DomainError;
use crate::domain::inference::services::provider_error::{
    ProviderErrorClass, classify_provider_error,
};

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

/// Await one read through `idle` that yields `line` after `after`.
async fn read_after(idle: &mut EventIdle, after: Duration, line: &'static str) -> Result<(), Idle> {
    let arriving = async move {
        tokio::time::sleep(after).await;
        Ok::<_, ()>(Some(line))
    };
    idle.next(arriving).await.map(|_| ())
}

/// Feed `idle` `line` every `every` until it ends, for at most `run`:
/// when it ended and its error.
async fn every(
    idle: &mut EventIdle,
    every: Duration,
    line: &'static str,
    run: Duration,
) -> Option<(Duration, Idle)> {
    let started = tokio::time::Instant::now();
    while started.elapsed() < run {
        if let Err(stalled) = read_after(idle, every, line).await {
            return Some((started.elapsed(), stalled));
        }
    }
    None
}

const PING: &str = "event: ping\ndata: {\"type\":\"ping\"}\n\n";

/// #2433 review round 3 (L1, L2): pings are events, so they restart the
/// idle bound, and 200 of them would take 50 minutes; the backstop ends a
/// ping-only think at three times the progress limit, 900 s, not 3000 s.
#[tokio::test(start_paused = true)]
async fn pings_every_fifteen_seconds_are_cut_by_the_backstop_at_fifteen_minutes() {
    let mut idle = EventIdle::events(StreamIdle::default());
    let ended = every(
        &mut idle,
        Duration::from_secs(15),
        PING,
        Duration::from_secs(3600),
    )
    .await;
    let (at, stalled) = ended.expect("a ping-only reply was cut");
    assert_eq!(at, Duration::from_secs(900));
    assert!(stalled.is_no_output(), "{stalled}");
    assert!(stalled.to_string().contains("backstop"), "{stalled}");
}

/// #2433 review round 3 (L2): a slow drip — an event every 299 s, never
/// tripping the idle bound or the event count — is cut by the backstop.
#[tokio::test(start_paused = true)]
async fn an_event_every_299_seconds_is_cut_by_the_backstop() {
    let mut idle = EventIdle::events(StreamIdle::default());
    let ended = every(
        &mut idle,
        Duration::from_secs(299),
        NO_OUTPUT,
        Duration::from_secs(3600),
    )
    .await;
    let (at, stalled) = ended.expect("the drip was cut");
    assert_eq!(at, Duration::from_secs(900));
    let message = stalled.to_string();
    assert!(
        message.starts_with("stream progress timeout: "),
        "{message}"
    );
    assert!(
        message.contains("900 s") && message.contains("backstop"),
        "{message}"
    );
}

/// Half an event a second reaches the event count at 400 s: the count rule
/// cuts it then, named as the progress limit.
#[tokio::test(start_paused = true)]
async fn half_an_event_a_second_is_cut_by_the_count_at_400_seconds() {
    let mut idle = EventIdle::events(StreamIdle::default());
    let ended = every(
        &mut idle,
        Duration::from_secs(2),
        NO_OUTPUT,
        Duration::from_secs(3600),
    )
    .await;
    let (at, stalled) = ended.expect("cut");
    assert_eq!(at, Duration::from_secs(400));
    assert_eq!(stalled, Idle::no_output(PROGRESS));
}

/// Silence stays the idle bound's: with the idle limit raised to 1800 s, a
/// silence after output, or after the opening events, ends only there.
#[tokio::test(start_paused = true)]
async fn silence_is_still_the_idle_bounds() {
    let idle_limit = Duration::from_secs(1800);
    for first in [REASONING, NO_OUTPUT] {
        let mut idle = EventIdle::events(StreamIdle::new(idle_limit));
        let started = tokio::time::Instant::now();
        read_after(&mut idle, Duration::ZERO, first).await.unwrap();
        let silent = idle.next(std::future::pending::<Result<Option<&str>, ()>>());
        assert_eq!(
            silent.await.unwrap_err(),
            Idle::no_event(idle_limit),
            "{first}"
        );
        assert_eq!(started.elapsed(), idle_limit, "{first}");
    }
}

/// #2433 review H2: a Codex think silent after its opening events is the
/// idle bound's alone: with the idle limit raised to 1800 s, a 600 s
/// silence is not cut.
#[tokio::test(start_paused = true)]
async fn a_silent_think_after_the_opening_events_is_governed_by_idle_only() {
    let mut idle = EventIdle::events(StreamIdle::new(Duration::from_secs(1800)));
    for opening in [
        "data: {\"type\":\"response.created\",\"response\":{}}\n\n",
        "data: {\"type\":\"response.in_progress\",\"response\":{}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"reasoning\",\"summary\":[]}}\n\n",
    ] {
        read_after(&mut idle, Duration::from_millis(50), opening)
            .await
            .unwrap();
    }
    let after_think = read_after(&mut idle, Duration::from_secs(600), REASONING).await;
    assert!(after_think.is_ok(), "{after_think:?}");
}

/// #2433 review M3: one output, then silence, ends as idle at the idle
/// bound, named so — never as a progress timeout.
#[tokio::test(start_paused = true)]
async fn output_then_silence_ends_as_idle_at_the_idle_bound() {
    let bounds = StreamIdle::new(Duration::from_secs(300)).with_progress(Duration::from_secs(60));
    let mut idle = EventIdle::events(bounds);
    let started = tokio::time::Instant::now();
    read_after(&mut idle, Duration::ZERO, REASONING)
        .await
        .unwrap();
    let silent = idle.next(std::future::pending::<Result<Option<&str>, ()>>());
    let ended = silent.await.unwrap_err();
    assert_eq!(started.elapsed(), Duration::from_secs(300));
    assert_eq!(ended, Idle::no_event(Duration::from_secs(300)));
}

/// An event `bytes` long of `kind`, its padding inside a string field.
fn long_event(kind: &str, field: &str, bytes: usize) -> &'static str {
    let padding = "x".repeat(bytes);
    let line = format!("data: {{\"type\":\"{kind}\",\"{field}\":\"{padding}\"}}\n\n");
    Box::leak(line.into_boxed_str())
}

/// #2433 review M4: an event too long to read whole is output only when
/// the type its start names is a delta or a finished part: a 70 KiB
/// `response.in_progress` ten times a second is cut by progress; a 70 KiB
/// text delta is not.
#[tokio::test(start_paused = true)]
async fn a_long_event_is_output_only_by_its_type() {
    let progress = Duration::from_secs(60);
    let bounds = StreamIdle::new(Duration::from_secs(1800)).with_progress(progress);
    let tick = Duration::from_millis(100);
    let in_progress = long_event("response.in_progress", "padding", 70 * 1024);
    let mut idle = EventIdle::events(bounds);
    let started = tokio::time::Instant::now();
    let ended = loop {
        if let Err(stalled) = read_after(&mut idle, tick, in_progress).await {
            break stalled;
        }
        assert!(started.elapsed() < progress * 2, "never cut");
    };
    assert_eq!(started.elapsed(), progress);
    assert_eq!(ended, Idle::no_output(progress));
    let delta = long_event("response.output_text.delta", "delta", 70 * 1024);
    let mut idle = EventIdle::events(bounds);
    for _ in 0..(progress.as_secs() * 15) {
        read_after(&mut idle, tick, delta).await.unwrap();
    }
}
