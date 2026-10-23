use serde_json::{Value, json};

use super::{paused_for, receipt};
use crate::application::swarm::dto::ControlReceipt;
use crate::application::swarm::services::board_operation::atomic;
use crate::application::swarm::test_support::{
    BoardState, MemoryBoard, SteppingClock, UsageReads, paused, recorded, running_board, usage,
};
use crate::domain::swarm::{
    events::telemetry::RefusalKind, value_objects::board_error::BoardError,
};

fn read(state: BoardState, now: f64) -> Result<ControlReceipt, BoardError> {
    let board = MemoryBoard::with(state);
    let clock = SteppingClock::fixed(now);
    atomic(&*board, false, |transaction| receipt(transaction, &*clock))
}

fn limited(limit: u64, observed: u64) -> Option<crate::application::swarm::dto::UsageReport> {
    Some(usage(
        json!({"token_limit": limit, "strict_unknown": false, "warned": false}),
        observed,
        0,
    ))
}

/// The receipt carries the run's status, outcome and reason, the control
/// generation, and the budget followed by the observed totals, in
/// Python's key order; a live run has no resume blockers.
#[test]
fn a_running_runs_receipt_merges_the_totals_into_the_budget() {
    let mut state = running_board(100.0);
    state.usage = limited(100, 30);
    let receipt = read(state, 50.0).unwrap();
    assert_eq!(
        (receipt.status.as_deref(), receipt.outcome, receipt.reason),
        (Some("running"), None, None)
    );
    assert_eq!(receipt.generation, 0);
    assert_eq!(
        Value::Object(receipt.budget).to_string(),
        r#"{"token_limit":100,"strict_unknown":false,"warned":false,"observed_tokens":30,"unknown_usage_requests":0}"#
    );
    assert!(receipt.resume_blockers.is_empty());
}

/// `resume_refuses_members_and_lists_blockers_for_the_supervisor` (the
/// blockers half): a paused run lists, in Python's order, a coordinator
/// lost after its latest activation and a deadline that the paused
/// interval does not carry past now. A spent budget stored before budgets
/// were dropped blocks nothing (owner decision, 2026-10-14).
#[test]
fn a_paused_runs_blockers_name_the_loss_and_the_deadline_never_a_budget() {
    let mut state = paused(running_board(40.0), 50.0, Some(("blocked", "why")));
    state.usage = limited(10, 10);
    state.events.push(recorded(
        "supervisor",
        "scope_unknown",
        json!({"member": "parent"}),
    ));
    let receipt = read(state.clone(), 200.0).unwrap();
    assert_eq!(
        (receipt.status.as_deref(), receipt.outcome.as_deref()),
        (Some("paused"), Some("blocked"))
    );
    assert_eq!(receipt.reason.as_deref(), Some("why"));
    assert_eq!(receipt.generation, 1, "the pause is the first event");
    assert_eq!(
        receipt.resume_blockers,
        [
            "relaunch the lost coordinator 'parent' into the retained environment before resuming",
            "extend the deadline (swarm_control extend) before resuming",
        ]
    );
    // Reactivated and carried past now by the paused interval, its budget
    // still spent: nothing blocks.
    state
        .events
        .push(recorded("parent", "activated", json!({"member": "parent"})));
    state.run.as_mut().unwrap().record.deadline = 100.0;
    let receipt = read(state, 189.0).unwrap();
    assert!(receipt.resume_blockers.is_empty(), "{receipt:?}");
}

/// A paused run whose pause was never recorded is refused with Python's
/// text; records only a file edited outside the board holds are refused
/// naming the record (`outside_edited_control_records`): a pause start
/// that is not a number (a boolean included, which Python counts as zero
/// or one). A budget and the usage totals are reported, never refused: no
/// budget decides on them any more (owner decision, 2026-10-14), so a
/// limit that is not a count, a total that is a REAL or negative, and a
/// budget that is no object (read as no budget) all read.
#[test]
fn missing_and_edited_control_records_are_refused() {
    let mut unrecorded = paused(running_board(40.0), 50.0, None);
    unrecorded.events.clear();
    assert_eq!(
        read(unrecorded, 60.0).unwrap_err(),
        BoardError::new(RefusalKind::Internal, "paused run has no pause record")
    );
    let mut text_start = paused(running_board(40.0), 50.0, None);
    text_start.events[0].detail = json!({"started": "soon"});
    // A budget that is no object reads as no budget (review round 1,
    // #2569): it is reported, never applied.
    for stored in [json!([]), json!(null), json!("x")] {
        let mut odd = running_board(40.0);
        odd.usage = Some(usage(stored.clone(), 0, 0));
        let receipt = read(odd, 30.0).unwrap();
        assert_eq!(
            Value::Object(receipt.budget).to_string(),
            r#"{"token_limit":null,"strict_unknown":false,"warned":false,"observed_tokens":0,"unknown_usage_requests":0}"#,
            "{stored}"
        );
    }
    let mut text_limit = paused(running_board(40.0), 50.0, None);
    text_limit.usage = Some(usage(
        json!({"token_limit": "10", "strict_unknown": false, "warned": false}),
        0,
        0,
    ));
    let text_limit = read(text_limit, 60.0).unwrap();
    assert_eq!(text_limit.budget["token_limit"], json!("10"));
    assert_eq!(
        text_limit.resume_blockers,
        ["extend the deadline (swarm_control extend) before resuming"],
        "the deadline alone"
    );
    let mut true_start = paused(running_board(40.0), 50.0, None);
    true_start.events[0].detail = json!({"started": true});
    let uncounted = |total: Value| {
        let mut state = paused(running_board(40.0), 50.0, None);
        let mut report = usage(
            json!({"token_limit": 100, "strict_unknown": false, "warned": false}),
            0,
            0,
        );
        for (column, value) in &mut report.totals.columns {
            if column == "observed_tokens" {
                *value = total.clone();
            }
        }
        state.usage = Some(report);
        state
    };
    for total in [json!(1.5), json!(-3)] {
        let receipt = read(uncounted(total.clone()), 60.0).unwrap();
        assert_eq!(receipt.budget["observed_tokens"], total);
    }
    for (state, record) in [(text_start, "pause record"), (true_start, "pause record")] {
        assert_eq!(
            read(state, 60.0).unwrap_err(),
            BoardError::new(
                RefusalKind::Store,
                format!("the board's {record} is not as the board writes it")
            )
        );
    }
}

/// `max(0, now - started)`: the integer 0 unless the run has been paused
/// for a positive interval.
#[test]
fn the_paused_interval_is_the_integer_zero_unless_positive() {
    assert_eq!(paused_for(10.0, 10.0), (0.0, json!(0)));
    assert_eq!(paused_for(10.0, 12.5), (0.0, json!(0)));
    assert_eq!(paused_for(12.5, 10.0), (2.5, json!(2.5)));
}

/// #2340: the receipt reads the ledger once, for the standing it reports,
/// and never the whole usage report; a paused run's blockers read no usage
/// at all.
#[test]
fn the_receipt_reads_the_usage_standing_once_and_never_the_whole_report() {
    let running = {
        let mut state = running_board(100.0);
        state.usage = limited(100, 30);
        state
    };
    let exhausted = {
        let mut state = paused(running_board(400.0), 50.0, None);
        state.usage = limited(10, 10);
        state
    };
    for (state, blockers) in [(running, 0), (exhausted, 0)] {
        let board = MemoryBoard::with(state);
        let clock = SteppingClock::fixed(60.0);
        let receipt = atomic(&*board, false, |transaction| receipt(transaction, &*clock)).unwrap();
        assert_eq!(receipt.resume_blockers.len(), blockers, "{receipt:?}");
        assert_eq!(
            board.snapshot().usage_reads,
            UsageReads {
                whole_reports: 0,
                standings: 1,
            },
            "{receipt:?}"
        );
    }
}
