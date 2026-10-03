//! #2436: each provider request a trace sees end is reported once, with the
//! usage it reported; an agent's tally numbers and sums them.
use super::*;
use std::sync::Mutex;

fn usage(input: u32, cached: Option<u32>, output: u32) -> UsageInfo {
    UsageInfo {
        prompt_tokens: input,
        completion_tokens: output,
        cache_read_tokens: cached,
        cache_write_tokens: None,
        context_tokens: None,
        cost: None,
    }
}

fn ended(outcome: RequestOutcome, spend: Option<RequestSpend>) -> EndedAttempt {
    EndedAttempt {
        attempt: 1,
        outcome,
        duration_ms: 5,
        spend,
    }
}

/// A trace whose ended attempts are kept, in the order they ended.
fn recorded_trace() -> (RequestTrace, Arc<Mutex<Vec<EndedAttempt>>>) {
    let trace = RequestTrace::default();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    trace.on_attempt_end(Arc::new(move |ended| sink.lock().unwrap().push(ended)));
    (trace, seen)
}

#[test]
fn the_tally_numbers_each_request_and_sums_what_was_reported() {
    let tally = RequestTally::default();
    let spend = RequestSpend {
        input_tokens: 100,
        cached_tokens: Some(40),
        output_tokens: 7,
    };
    let first = tally.record("gpt-5.5", "codex", ended(RequestOutcome::Ok, Some(spend)));
    let second = tally.record("gpt-5.5", "codex", ended(RequestOutcome::Error, None));
    let third = tally.record(
        "gpt-5.5",
        "codex",
        ended(
            RequestOutcome::Ok,
            Some(RequestSpend {
                cached_tokens: None,
                ..spend
            }),
        ),
    );
    assert_eq!(
        [
            first.request_index,
            second.request_index,
            third.request_index
        ],
        [1, 2, 3]
    );
    assert_eq!(
        (first.model.as_str(), first.provider.as_str()),
        ("gpt-5.5", "codex")
    );
    assert_eq!(second.spend, None, "nothing reported stays unknown");
    assert_eq!(
        tally.counters(),
        AgentRequestCounters {
            requests: 3,
            input_tokens: 200,
            cached_tokens: 40,
            output_tokens: 14,
        }
    );
}

#[test]
fn two_tallies_never_share_a_count() {
    let (mine, theirs) = (RequestTally::default(), RequestTally::default());
    for _ in 0..5 {
        theirs.record("m", "p", ended(RequestOutcome::Ok, None));
    }
    let record = mine.record("m", "p", ended(RequestOutcome::Ok, None));
    assert_eq!(record.request_index, 1);
    assert_eq!(mine.counters().requests, 1);
    assert_eq!(theirs.counters().requests, 5);
}

#[test]
fn a_spend_is_known_only_from_a_report() {
    assert_eq!(RequestSpend::of([]), None);
    let reports = [usage(10, None, 1), usage(20, Some(5), 2)];
    assert_eq!(
        RequestSpend::of(&reports),
        Some(RequestSpend {
            input_tokens: 30,
            cached_tokens: Some(5),
            output_tokens: 3,
        })
    );
    assert_eq!(
        RequestSpend::of(&[usage(10, None, 1)]).and_then(|spend| spend.cached_tokens),
        None,
        "a provider that never reported cache reads leaves them unknown"
    );
}

#[test]
fn each_attempt_is_reported_once_with_its_own_usage() {
    let (trace, seen) = recorded_trace();
    trace.start();
    trace.record_unfinished_usage(usage(300, Some(100), 4));
    trace.end_attempt_failed();
    trace.end_attempt_failed();
    trace.retry();
    trace.end_attempt(RequestOutcome::Ok, Some(&usage(50, Some(250), 9)));
    trace.end_attempt(RequestOutcome::Ok, None);
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2, "one report per attempt: {seen:?}");
    assert_eq!(
        (seen[0].attempt, seen[0].outcome),
        (1, RequestOutcome::Error)
    );
    assert_eq!(
        seen[0].spend,
        Some(RequestSpend {
            input_tokens: 300,
            cached_tokens: Some(100),
            output_tokens: 4,
        }),
        "the cut-short attempt's usage is its own"
    );
    assert_eq!((seen[1].attempt, seen[1].outcome), (2, RequestOutcome::Ok));
    assert_eq!(
        seen[1].spend,
        Some(RequestSpend {
            input_tokens: 50,
            cached_tokens: Some(250),
            output_tokens: 9,
        }),
        "the retry reports its reply's usage, not its predecessor's"
    );
}

#[test]
fn a_retry_ends_an_attempt_nobody_ended() {
    let (trace, seen) = recorded_trace();
    trace.start();
    trace.start();
    trace.oauth_retry();
    trace.end_attempt(RequestOutcome::Cancelled, None);
    let seen = seen.lock().unwrap().clone();
    let reported: Vec<_> = seen.iter().map(|e| (e.attempt, e.outcome)).collect();
    assert_eq!(
        reported,
        [(1, RequestOutcome::Error), (2, RequestOutcome::Cancelled)]
    );
    assert!(seen.iter().all(|e| e.spend.is_none()));
}

#[test]
fn a_request_that_never_started_an_attempt_reports_none() {
    let (trace, seen) = recorded_trace();
    trace.end_attempt(RequestOutcome::Error, None);
    trace.end_attempt_failed();
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn outcomes_have_their_wire_names() {
    for outcome in [
        RequestOutcome::Ok,
        RequestOutcome::Error,
        RequestOutcome::Cancelled,
    ] {
        assert_eq!(
            serde_json::to_value(outcome).unwrap(),
            serde_json::Value::String(outcome.as_str().into())
        );
    }
    assert_eq!(RequestOutcome::Cancelled.as_str(), "cancelled");
}
