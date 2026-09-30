//! #2338: the run watch's unchanged cursor polls are folded into one
//! `swarm_op` record per run of them, counted in `polls`; every other poll
//! is recorded alone, and no poll is lost.
use super::*;
use crate::domain::swarm::{BoardOpDetail, BoardRole, RefusalKind};

const RUN: &str = "0123456789abcdef0123456789abcdef";

/// An answered watch poll, not slowed by the busy handler, of `duration_us`.
fn poll(duration_us: u64) -> BoardOpObservation {
    BoardOpObservation {
        op: WATCH_POLL_OP.into(),
        actor_ref: "worker".into(),
        role: Some(BoardRole::Host),
        run_id: Some(RUN.into()),
        task_id: None,
        message_id: None,
        outcome: BoardOpOutcome::Ok,
        duration_us,
        lock_wait_us: Some(duration_us / 4),
        busy_wait_us: Some(0),
        busy: Some(false),
        cursor_moved: None,
        result_bytes: 1,
        decision: Some("read".into()),
        detail: BoardOpDetail::NONE,
    }
}

/// The polls every written record accounts for: an aggregate's `polls`,
/// else one.
fn accounted(records: &[BoardOpObservation]) -> u64 {
    records
        .iter()
        .map(|record| record.detail.polls.unwrap_or(1))
        .sum()
}

#[test]
fn unchanged_polls_are_folded_into_one_record_counting_them() {
    let mut tally = PollTally::default();
    let mut written = tally.poll(poll(10), Some(7), 0);
    assert_eq!(written.len(), 1, "the first poll is recorded alone");
    for (at, duration) in [(500_000, 30), (1_000_000, 20), (1_500_000, 5)] {
        written.extend(tally.poll(poll(duration), Some(7), at));
    }
    assert_eq!(written.len(), 1, "unchanged polls are held: {written:?}");
    assert_eq!(tally.pending_polls(), 3);
    let aggregate = tally
        .flush()
        .expect("the held polls are written on a flush");
    assert_eq!(aggregate.op, WATCH_POLL_OP);
    assert_eq!(aggregate.decision.as_deref(), Some(UNCHANGED));
    assert_eq!(aggregate.detail.polls, Some(3));
    assert_eq!(aggregate.cursor_moved, Some(false));
    assert_eq!(aggregate.busy, Some(false));
    assert_eq!(aggregate.duration_us, 30, "the slowest poll's duration");
    assert_eq!(
        aggregate.lock_wait_us,
        Some(7),
        "the slowest poll's lock wait"
    );
    assert_eq!(aggregate.run_id.as_deref(), Some(RUN));
    assert_eq!(tally.pending_polls(), 0);
    assert!(tally.flush().is_none(), "flushed once");
}

#[test]
fn a_moved_cursor_writes_the_held_polls_then_itself() {
    let mut tally = PollTally::default();
    let _first = tally.poll(poll(10), Some(7), 0);
    assert!(tally.poll(poll(10), Some(7), 1).is_empty());
    assert!(tally.poll(poll(10), Some(7), 2).is_empty());
    let written = tally.poll(poll(10), Some(8), 3);
    assert_eq!(written.len(), 2, "{written:?}");
    assert_eq!(written[0].decision.as_deref(), Some(UNCHANGED));
    assert_eq!(written[0].detail.polls, Some(2));
    assert_eq!(
        written[1].decision.as_deref(),
        Some("read"),
        "the moved poll is recorded as any read is"
    );
    assert_eq!(written[1].detail.polls, None, "a single call, no aggregate");
}

#[test]
fn a_busy_or_refused_poll_is_never_absorbed() {
    let mut tally = PollTally::default();
    let _first = tally.poll(poll(10), Some(7), 0);
    let busy = BoardOpObservation {
        busy: Some(true),
        busy_wait_us: Some(40),
        ..poll(50)
    };
    let written = tally.poll(busy, Some(7), 1);
    assert_eq!(written.len(), 1, "a busy poll is recorded alone");
    assert_eq!(written[0].busy, Some(true));
    let refused = BoardOpObservation {
        outcome: BoardOpOutcome::Refused {
            kind: RefusalKind::Contended,
            committed: false,
        },
        decision: None,
        ..poll(500_000)
    };
    let written = tally.poll(refused, None, 2);
    assert_eq!(written.len(), 1, "a refused poll is recorded alone");
    assert!(
        tally.poll(poll(10), Some(7), 3).is_empty(),
        "the cursor the last answered poll read is still the one compared"
    );
}

#[test]
fn a_poll_of_another_run_is_not_folded_with_the_last() {
    let mut tally = PollTally::default();
    let _first = tally.poll(poll(10), Some(7), 0);
    assert!(tally.poll(poll(10), Some(7), 1).is_empty());
    let other = BoardOpObservation {
        run_id: Some("f".repeat(32)),
        ..poll(10)
    };
    let written = tally.poll(other, Some(7), 2);
    assert_eq!(written.len(), 2, "the held poll, then the other run's");
    assert_eq!(written[1].run_id.as_deref(), Some("f".repeat(32).as_str()));
}

#[test]
fn an_aggregate_is_written_once_it_has_been_open_for_the_period() {
    let mut tally = PollTally::default();
    let mut written = tally.poll(poll(10), Some(7), 0);
    let mut at = 0;
    while at < 3 * POLL_RECORD_PERIOD_US {
        at += 500_000;
        written.extend(tally.poll(poll(10), Some(7), at));
    }
    let aggregates = written
        .iter()
        .filter(|record| record.detail.polls.is_some())
        .count();
    assert!(
        (2..=3).contains(&aggregates),
        "about one aggregate per period: {aggregates}"
    );
    written.extend(tally.flush());
    assert_eq!(
        accounted(&written),
        1 + 3 * POLL_RECORD_PERIOD_US / 500_000,
        "every poll is accounted for, counted if not recorded"
    );
}

/// An idle minute of the watch at its tick writes a handful of records,
/// not one per poll, and still accounts for all of them.
#[test]
fn an_idle_minute_is_a_few_records_accounting_for_every_poll() {
    let mut tally = PollTally::default();
    let mut written = Vec::new();
    for tick in 0..120_u64 {
        written.extend(tally.poll(poll(10), Some(7), tick * 500_000));
    }
    written.extend(tally.flush());
    assert!(written.len() <= 3, "{} records", written.len());
    assert_eq!(accounted(&written), 120);
}

/// Nothing but what the poll carried is written: the aggregate keeps the
/// poll's (already redacted) actor ref and adds only counts and kinds.
#[test]
fn an_aggregate_carries_no_text_beyond_the_polls_own() {
    let mut tally = PollTally::default();
    let secret_shaped = BoardOpObservation {
        actor_ref: "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789".into(),
        ..poll(10)
    };
    let _first = tally.poll(secret_shaped.clone(), Some(7), 0);
    assert!(tally.poll(secret_shaped.clone(), Some(7), 1).is_empty());
    let aggregate = tally.flush().unwrap();
    let text = serde_json::to_string(&aggregate).unwrap();
    assert!(
        !text.contains("sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789"),
        "the actor ref is the poll's redacted one: {text}"
    );
    assert!(text.contains("\"polls\":2"), "{text}");
}
