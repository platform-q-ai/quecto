//! #2338: the run watch's `unchanged` ticks are folded into one
//! `swarm_op` record per run of them, counted in `polls`; every other tick
//! is recorded alone, and no tick is lost.
use super::*;
use crate::domain::swarm::{BoardOpDetail, BoardRole, RefusalKind};

const RUN: &str = "0123456789abcdef0123456789abcdef";

/// A watch tick that answered the snapshot.
fn snapshot(duration_us: u64) -> BoardOpObservation {
    BoardOpObservation {
        decision: Some(SNAPSHOT.into()),
        ..poll(duration_us)
    }
}

/// An `unchanged` watch tick, not slowed by the busy handler, of
/// `duration_us`.
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
        decision: Some(UNCHANGED.into()),
        detail: BoardOpDetail::NONE,
        arguments: Box::new(crate::domain::swarm::ArgumentFaults::NONE),
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
fn unchanged_ticks_are_folded_into_one_record_counting_them() {
    let mut tally = PollTally::default();
    let mut written = tally.poll(snapshot(10), 0);
    assert_eq!(written.len(), 1, "a snapshot tick is recorded alone");
    for (at, duration) in [(500_000, 30), (1_000_000, 20), (1_500_000, 5)] {
        written.extend(tally.poll(poll(duration), at));
    }
    assert_eq!(written.len(), 1, "unchanged ticks are held: {written:?}");
    assert_eq!(tally.pending_polls(), 3);
    let aggregate = tally
        .flush()
        .expect("the held ticks are written on a flush");
    assert_eq!(aggregate.op, WATCH_POLL_OP);
    assert_eq!(aggregate.decision.as_deref(), Some(UNCHANGED));
    assert_eq!(aggregate.detail.polls, Some(3));
    assert_eq!(aggregate.cursor_moved, Some(false));
    assert_eq!(aggregate.busy, Some(false));
    assert_eq!(aggregate.duration_us, 30, "the slowest tick's duration");
    assert_eq!(
        aggregate.lock_wait_us,
        Some(7),
        "the slowest tick's lock wait"
    );
    assert_eq!(aggregate.run_id.as_deref(), Some(RUN));
    assert_eq!(tally.pending_polls(), 0);
    assert!(tally.flush().is_none(), "flushed once");
}

#[test]
fn a_snapshot_tick_writes_the_held_ticks_then_itself() {
    let mut tally = PollTally::default();
    assert!(tally.poll(poll(10), 1).is_empty());
    assert!(tally.poll(poll(10), 2).is_empty());
    let written = tally.poll(snapshot(10), 3);
    assert_eq!(written.len(), 2, "{written:?}");
    assert_eq!(written[0].decision.as_deref(), Some(UNCHANGED));
    assert_eq!(written[0].detail.polls, Some(2));
    assert_eq!(written[1].decision.as_deref(), Some(SNAPSHOT));
    assert_eq!(written[1].detail.polls, None, "a single call, no aggregate");
}

#[test]
fn a_busy_or_refused_tick_is_never_absorbed() {
    let mut tally = PollTally::default();
    assert!(tally.poll(poll(10), 0).is_empty());
    let busy = BoardOpObservation {
        busy: Some(true),
        busy_wait_us: Some(40),
        ..poll(50)
    };
    let written = tally.poll(busy, 1);
    assert_eq!(written.len(), 2, "the held tick, then the busy one alone");
    assert_eq!(written[1].busy, Some(true));
    let refused = BoardOpObservation {
        outcome: BoardOpOutcome::Refused {
            kind: RefusalKind::Contended,
            committed: false,
        },
        decision: None,
        ..poll(500_000)
    };
    let written = tally.poll(refused, 2);
    assert_eq!(written.len(), 1, "a refused tick is recorded alone");
    assert!(tally.poll(poll(10), 3).is_empty(), "held again");
}

#[test]
fn a_tick_of_another_run_is_not_folded_with_the_held_ones() {
    let mut tally = PollTally::default();
    assert!(tally.poll(poll(10), 1).is_empty());
    let other = BoardOpObservation {
        run_id: Some("f".repeat(32)),
        ..poll(10)
    };
    let written = tally.poll(other, 2);
    assert_eq!(written.len(), 1, "the held tick is written");
    assert_eq!(written[0].run_id.as_deref(), Some(RUN));
    assert_eq!(tally.pending_polls(), 1, "the other run's is held");
    let held = tally.flush().unwrap();
    assert_eq!(held.run_id.as_deref(), Some("f".repeat(32).as_str()));
}

#[test]
fn an_aggregate_is_written_once_it_has_been_open_for_the_period() {
    let mut tally = PollTally::default();
    let mut written = Vec::new();
    let mut at = 0;
    while at < 3 * POLL_RECORD_PERIOD_US {
        written.extend(tally.poll(poll(10), at));
        at += 500_000;
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
        3 * POLL_RECORD_PERIOD_US / 500_000,
        "every tick is accounted for, counted if not recorded"
    );
}

/// An idle minute of the watch at its tick writes a handful of records,
/// not one per tick, and still accounts for all of them.
#[test]
fn an_idle_minute_is_a_few_records_accounting_for_every_tick() {
    let mut tally = PollTally::default();
    let mut written = Vec::new();
    for tick in 0..120_u64 {
        written.extend(tally.poll(poll(10), tick * 500_000));
    }
    written.extend(tally.flush());
    assert!(written.len() <= 3, "{} records", written.len());
    assert_eq!(accounted(&written), 120);
}

/// Nothing but what the ticks carried is written: the aggregate keeps the
/// tick's (already redacted) actor ref and adds only counts and kinds.
#[test]
fn an_aggregate_carries_no_text_beyond_the_ticks_own() {
    let mut tally = PollTally::default();
    let secret_shaped = BoardOpObservation {
        actor_ref: "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789".into(),
        ..poll(10)
    };
    assert!(tally.poll(secret_shaped.clone(), 1).is_empty());
    assert!(tally.poll(secret_shaped.clone(), 2).is_empty());
    let aggregate = tally.flush().unwrap();
    let text = serde_json::to_string(&aggregate).unwrap();
    assert!(
        !text.contains("sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789"),
        "the actor ref is the tick's redacted one: {text}"
    );
    assert!(text.contains("\"polls\":2"), "{text}");
}
