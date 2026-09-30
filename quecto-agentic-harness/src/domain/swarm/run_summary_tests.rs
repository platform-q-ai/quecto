//! #2313: the run summary is a pure fold of the run's `swarm_op` records:
//! its counts are theirs, its percentiles leave out what was not measured,
//! and it carries ids, kinds, durations and sizes only.
use serde_json::{Value, json};

use super::*;
use crate::domain::audit::AuditEvent;
use crate::domain::swarm::telemetry::{BoardOpDetail, BoardRole};
use crate::domain::swarm::{RunStatus, Snapshot};

const RUN: &str = "0123456789abcdef0123456789abcdef";

/// An answered `op` record of the run, by `actor`, that decided `decision`
/// and took `duration_us`, its waits `lock` and `busy_wait`.
fn record(op: &str, decision: Option<&str>, duration_us: u64) -> BoardOpObservation {
    BoardOpObservation {
        op: op.into(),
        actor_ref: "worker".into(),
        role: Some(BoardRole::Worker),
        run_id: Some(RUN.into()),
        task_id: None,
        message_id: None,
        outcome: BoardOpOutcome::Ok,
        duration_us,
        lock_wait_us: Some(duration_us / 2),
        busy_wait_us: Some(0),
        busy: Some(false),
        commit_us: Some(duration_us / 4),
        cursor_moved: None,
        result_bytes: 0,
        decision: decision.map(str::to_owned),
        detail: BoardOpDetail::NONE,
        arguments: Box::new(crate::domain::swarm::ArgumentFaults::NONE),
    }
}

fn refused(op: &str, kind: RefusalKind) -> BoardOpObservation {
    BoardOpObservation {
        outcome: BoardOpOutcome::Refused {
            kind,
            committed: false,
        },
        decision: None,
        lock_wait_us: None,
        busy_wait_us: None,
        busy: None,
        commit_us: None,
        ..record(op, None, 10)
    }
}

fn folded(records: &[BoardOpObservation]) -> SwarmRunSummary {
    let mut fold = RunSummaryFold::new(RUN, 1_000);
    for record in records {
        fold.observe(record);
    }
    fold.summary(9_000, None)
}

/// The counts by `(op, kind)` are the records': each answered record once
/// in its op's `ok`, each refusal once under its op and kind.
#[test]
fn the_summary_counts_each_record_under_its_op_and_kind() {
    let summary = folded(&[
        record("claim", Some("claimed"), 10),
        record("claim", Some("claimed"), 20),
        refused("claim", RefusalKind::WrongState),
        refused("claim", RefusalKind::WrongState),
        refused("claim", RefusalKind::NotMember),
        record("summary", Some("read"), 30),
    ]);
    assert_eq!(summary.run_id, RUN);
    assert_eq!(summary.records, 6);
    let claim = &summary.ops["claim"];
    assert_eq!(claim.ok, 2);
    assert_eq!(claim.refused[&RefusalKind::WrongState], 2);
    assert_eq!(claim.refused[&RefusalKind::NotMember], 1);
    assert_eq!(summary.ops["summary"].ok, 1);
    assert!(summary.ops["summary"].refused.is_empty());
    assert_eq!(summary.ops.len(), 2);
}

/// The percentiles are nearest-rank over the measured records; a record
/// that measured nothing (`null`) is left out and counted as unmeasured.
#[test]
fn the_percentiles_leave_out_what_was_not_measured() {
    let mut records: Vec<_> = (1..=20)
        .map(|n| record("claim", Some("claimed"), n * 100))
        .collect();
    records.push(refused("claim", RefusalKind::WrongState));
    let summary = folded(&records);
    let claim = &summary.ops["claim"];
    assert_eq!(
        claim.duration_us,
        Percentiles {
            p50: Some(1_000),
            p95: Some(1_900),
            max: Some(2_000),
            unmeasured: 0,
        },
        "the refusal's 10 µs is a duration too"
    );
    assert_eq!(
        claim.lock_wait_us,
        Percentiles {
            p50: Some(500),
            p95: Some(950),
            max: Some(1_000),
            unmeasured: 1,
        }
    );
    assert_eq!(claim.busy_wait_us.unmeasured, 1);
    assert_eq!(claim.busy_wait_us.max, Some(0));
}

/// #2340: an op's commit times are summarised as its waits are: the
/// nearest-rank p50 and p95 and the max of the records that measured
/// one, the others counted as unmeasured; a zero (no `COMMIT` ran) is a
/// measure, not a gap.
#[test]
fn commit_times_are_summarised_as_the_waits_are() {
    let mut records: Vec<_> = (1..=20)
        .map(|n| record("claim", Some("claimed"), n * 100))
        .collect();
    records.push(refused("claim", RefusalKind::WrongState));
    records.push(BoardOpObservation {
        commit_us: Some(0),
        ..refused("claim", RefusalKind::WrongState)
    });
    let summary = folded(&records);
    assert_eq!(
        summary.ops["claim"].commit_us,
        Percentiles {
            p50: Some(250),
            p95: Some(475),
            max: Some(500),
            unmeasured: 1,
        }
    );
    let line = serde_json::to_value(&summary.ops["claim"]).unwrap();
    assert_eq!(
        line["commit_us"],
        serde_json::json!({"p50": 250, "p95": 475, "max": 500, "unmeasured": 1}),
        "{line}"
    );
}

/// An op none of whose records measured a wait has no percentile of it,
/// only the count of those records.
#[test]
fn an_op_that_measured_nothing_has_no_percentile() {
    let summary = folded(&[refused("resume", RefusalKind::NotCoordinator)]);
    let resume = &summary.ops["resume"];
    assert_eq!(
        resume.lock_wait_us,
        Percentiles {
            p50: None,
            p95: None,
            max: None,
            unmeasured: 1,
        }
    );
    assert_eq!(resume.duration_us.max, Some(10));
}

/// The busy count is the records whose busy handler fired.
#[test]
fn the_busy_count_is_the_records_that_found_the_store_busy() {
    let busy = BoardOpObservation {
        busy: Some(true),
        busy_wait_us: Some(40),
        ..record("claim", Some("claimed"), 50)
    };
    let summary = folded(&[busy.clone(), busy, record("send", Some("sent"), 5)]);
    assert_eq!(summary.busy, 2);
    assert_eq!(summary.ops["claim"].busy, 2);
    assert_eq!(summary.ops["send"].busy, 0);
}

/// Tasks and messages are counted by what an answered op decided: a
/// replayed create, an unchanged block or a refusal changes nothing.
#[test]
fn tasks_and_messages_are_counted_by_the_decision_taken() {
    let summary = folded(&[
        record("task_create", Some("created"), 1),
        record("task_create", Some("created"), 1),
        record("task_create", Some("replayed"), 1),
        record("claim", Some("claimed"), 1),
        record("release", Some("released"), 1),
        record("block", Some("blocked"), 1),
        record("block", Some("unchanged"), 1),
        record("submit", Some("submitted"), 1),
        record("verify_task", Some("verified"), 1),
        refused("verify_task", RefusalKind::StaleRevision),
        record("send", Some("sent"), 1),
        record("send", Some("replayed"), 1),
        record("ack", Some("consumed"), 1),
        record("ack", Some("unchanged"), 1),
        record("withdraw", Some("withdrawn"), 1),
    ]);
    assert_eq!(
        summary.tasks,
        TaskCounts {
            created: 2,
            claimed: 1,
            released: 1,
            blocked: 1,
            submitted: 1,
            accepted: 1,
        }
    );
    assert_eq!(
        summary.messages,
        MessageCounts {
            sent: 1,
            acked: 1,
            withdrawn: 1,
        }
    );
}

/// Each member's recorded requests (`_record_request`), recorded or
/// refused, by its redacted ref: a member id shaped like a credential is
/// never written.
#[test]
fn request_usage_is_counted_per_member_by_its_redacted_ref() {
    let secret = "sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    let by = |actor: &str, observation: BoardOpObservation| BoardOpObservation {
        actor_ref: actor.into(),
        ..observation
    };
    let summary = folded(&[
        by("worker", record("_record_request", Some("recorded"), 1)),
        by("worker", record("_record_request", Some("warned"), 1)),
        by(
            "worker",
            refused("_record_request", RefusalKind::RequestIdReused),
        ),
        by(secret, record("_record_request", Some("recorded"), 1)),
        by("worker", record("claim", Some("claimed"), 1)),
    ]);
    assert_eq!(summary.request_usage.len(), 2);
    assert_eq!(summary.request_usage[0].actor_ref.as_str(), "worker");
    assert_eq!(
        (
            summary.request_usage[0].recorded,
            summary.request_usage[0].refused
        ),
        (2, 1)
    );
    assert_eq!(summary.request_usage[1].recorded, 1);
    let text = serde_json::to_string(&summary).unwrap();
    assert!(!text.contains("sk-ant"), "{text}");
}

/// The process's own span runs from the start of the first record it
/// folded to the moment the summary is taken (#2313 review L3: the run's
/// wall time is the run-wide section's, from the run's creation).
#[test]
fn the_process_span_runs_from_its_first_record_to_the_summary() {
    let mut fold = RunSummaryFold::new(RUN, 250);
    fold.observe(&record("claim", Some("claimed"), 50));
    assert_eq!(fold.summary(10_250, None).process_span_us, 10_000);
    assert_eq!(fold.summary(100, None).process_span_us, 0, "never negative");
}

/// A long run keeps at most [`SAMPLES_PER_OP`] samples of an op, drawn
/// uniformly over all its records (#2313 review L2): its counts and its
/// max stay exact, the records not held are counted, and a record late in
/// the run is as likely to be held as an early one. Every record here
/// took a distinct time, so what was kept shows in the percentiles.
#[test]
fn an_op_keeps_a_uniform_bounded_sample_and_an_exact_max() {
    let mut fold = RunSummaryFold::new(RUN, 0);
    let total = 2 * SAMPLES_PER_OP as u64;
    for duration in 1..=total {
        fold.observe(&record("summary", Some("read"), duration));
    }
    let summary = fold.summary(1, None);
    let op = &summary.ops["summary"];
    assert_eq!(op.ok, total);
    assert_eq!(op.unsampled, total - SAMPLES_PER_OP as u64);
    assert_eq!(op.duration_us.max, Some(total), "the max is exact");
    assert_eq!(op.lock_wait_us.max, Some(total / 2), "every measure's is");
    assert_eq!(op.commit_us.max, Some(total / 4), "the commit time's too");
    let p50 = op.duration_us.p50.unwrap();
    let p95 = op.duration_us.p95.unwrap();
    // The first records alone would give p50 near total/4 and p95 near
    // total/2; a uniform sample of every record gives total/2 and 0.95
    // total, within a few percent.
    let near = |value: u64, expected: u64| value.abs_diff(expected) * 20 < total;
    assert!(near(p50, total / 2), "p50 {p50} of {total}");
    assert!(near(p95, total * 95 / 100), "p95 {p95} of {total}");
}

/// The same records give the same summary: the sample is drawn from a
/// seed the run's id fixes, never from the clock.
#[test]
fn the_sample_is_deterministic_for_a_run() {
    let fold = || {
        let mut fold = RunSummaryFold::new(RUN, 0);
        for duration in 1..=(3 * SAMPLES_PER_OP as u64) {
            fold.observe(&record("claim", Some("claimed"), duration));
        }
        fold.summary(1, None)
    };
    assert_eq!(fold(), fold());
}

/// Records of an op past [`SUMMARY_OPS`] distinct ops are counted in
/// `unlisted_ops` and in `records`, and in no op's entry.
#[test]
fn records_of_ops_past_the_bound_are_counted_unlisted() {
    let records: Vec<_> = (0..SUMMARY_OPS + 2)
        .map(|n| record(&format!("op{n}"), Some("read"), 1))
        .collect();
    let summary = folded(&records);
    assert_eq!(summary.ops.len(), SUMMARY_OPS);
    assert_eq!(summary.unlisted_ops, 2);
    assert_eq!(summary.records, SUMMARY_OPS as u64 + 2);
    let again = folded(&[records.clone(), records].concat());
    assert_eq!(again.unlisted_ops, 4, "an unlisted op stays unlisted");
}

/// Requests of members past [`REQUEST_MEMBERS`] are counted in
/// `unlisted_requests`, and still under `ops._record_request`.
#[test]
fn requests_of_members_past_the_bound_are_counted_unlisted() {
    let records: Vec<_> = (0..REQUEST_MEMBERS + 3)
        .map(|n| BoardOpObservation {
            actor_ref: format!("member{n}").as_str().into(),
            ..record("_record_request", Some("recorded"), 1)
        })
        .collect();
    let summary = folded(&records);
    assert_eq!(summary.request_usage.len(), REQUEST_MEMBERS);
    assert_eq!(summary.unlisted_requests, 3);
    assert_eq!(
        summary.ops["_record_request"].ok,
        REQUEST_MEMBERS as u64 + 3
    );
}

/// The record is `swarm_run_summary`, holding counts, kinds, durations and
/// the run's id, and reads back as it was written.
#[test]
fn a_run_summary_record_round_trips() {
    let summary = folded(&[
        record("claim", Some("claimed"), 10),
        refused("claim", RefusalKind::WrongState),
    ]);
    let event = AuditEvent::SwarmRunSummary(summary.clone());
    let line = serde_json::to_value(&event).unwrap();
    assert_eq!(line["event"], "swarm_run_summary");
    assert_eq!(line["run_id"], RUN);
    assert_eq!(line["records"], 2);
    assert_eq!(line["ops"]["claim"]["ok"], 1);
    assert_eq!(line["ops"]["claim"]["refused"], json!({"wrong_state": 1}));
    assert_eq!(line["tasks"]["claimed"], 1);
    assert_eq!(line["scope"], "process", "the fold is this process's");
    assert_eq!(line["process_span_us"], 8_000);
    assert_eq!(line["wall_time_us"], Value::Null, "the run's is run-wide");
    assert_eq!(line["run"], Value::Null, "no board read given");
    assert_eq!(line["unlisted_ops"], Value::Null, "left out when none");
    let read: AuditEvent = serde_json::from_value(line).unwrap();
    assert_eq!(read, event);
}

/// #2313 review M1: the board's run-wide totals, read at settle, are the
/// record's `run` section, beside the process's own counts.
#[test]
fn the_run_wide_totals_are_the_records_run_section() {
    let mut fold = RunSummaryFold::new(RUN, 0);
    fold.observe(&record("summary", Some("full"), 5));
    let totals = crate::domain::swarm::RunTotals::new(
        crate::domain::swarm::TaskStates {
            total: 3,
            completed: 3,
            ..Default::default()
        },
        crate::domain::swarm::MessageTotals::default(),
        Vec::new(),
        Some(10.0),
        12.0,
    );
    let summary = fold.summary(5, Some(totals.clone()));
    assert_eq!(summary.run, Some(totals));
    let line = serde_json::to_value(AuditEvent::SwarmRunSummary(summary)).unwrap();
    assert_eq!(line["scope"], "process");
    assert_eq!(line["records"], 1);
    assert_eq!(line["run"]["tasks"]["completed"], 3);
    assert_eq!(line["run"]["wall_time_us"], 2_000_000);
}

/// The coordinator's harness writes the summary once its run settled
/// (ended, or cancelled); no other member's does, and a live or paused
/// run has none yet.
#[test]
fn only_the_coordinator_of_a_settled_run_writes_its_summary() {
    let snapshot = |status| Snapshot {
        control_generation: 0,
        status,
        outcome: None,
        coordinator: "parent".into(),
        deadline: 0.0,
        members: Vec::new(),
    };
    for status in [
        RunStatus::Succeeded,
        RunStatus::Blocked,
        RunStatus::Failed,
        RunStatus::Cancelled,
        RunStatus::BudgetExhausted,
    ] {
        assert!(snapshot(status).summarized_by("parent"), "{status:?}");
        assert!(!snapshot(status).summarized_by("worker"), "{status:?}");
    }
    for status in [RunStatus::Setup, RunStatus::Running, RunStatus::Paused] {
        assert!(!snapshot(status).summarized_by("parent"), "{status:?}");
    }
}

/// #2338: an aggregate of the watch's unchanged cursor polls is one record
/// counting `polls` calls: the op's `ok` and the summary's `calls` count
/// every poll, `records` the lines, and the aggregate adds one sample (its
/// slowest poll's) to the percentiles.
#[test]
fn an_aggregate_of_unchanged_polls_counts_every_poll_it_holds() {
    let aggregate = BoardOpObservation {
        decision: Some("unchanged".into()),
        cursor_moved: Some(false),
        detail: BoardOpDetail {
            polls: Some(118),
            ..BoardOpDetail::NONE
        },
        ..record("_watch", Some("unchanged"), 40)
    };
    let summary = folded(&[
        record("_watch", Some("snapshot"), 10),
        aggregate,
        record("_watch", Some("snapshot"), 20),
        record("_snapshot", Some("read"), 30),
    ]);
    assert_eq!(summary.records, 4, "the lines folded");
    assert_eq!(summary.calls, 121, "every board call the lines account for");
    let polls = &summary.ops["_watch"];
    assert_eq!(polls.ok, 120, "each poll counted, recorded or aggregated");
    assert_eq!(polls.duration_us.max, Some(40));
    assert_eq!(polls.duration_us.unmeasured, 0);
    assert_eq!(summary.ops["_snapshot"].ok, 1);
    let line = serde_json::to_value(AuditEvent::SwarmRunSummary(summary)).unwrap();
    assert_eq!(line["calls"], 121);
}

/// A summary written before #2338 (no `calls`) still reads back.
#[test]
fn a_summary_without_calls_reads_back() {
    let summary = folded(&[record("claim", Some("claimed"), 10)]);
    let mut line = serde_json::to_value(AuditEvent::SwarmRunSummary(summary)).unwrap();
    line.as_object_mut().unwrap().remove("calls");
    let read: AuditEvent = serde_json::from_value(line).unwrap();
    assert!(matches!(read, AuditEvent::SwarmRunSummary(summary) if summary.calls == 0));
}
