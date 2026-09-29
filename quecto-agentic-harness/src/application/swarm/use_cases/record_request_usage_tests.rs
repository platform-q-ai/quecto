use serde_json::{Value, json};

use super::RecordRequestUsage;
use crate::application::swarm::board_test_support::{
    BoardState, CompactEncoding, MemoryBoard, SteppingClock, UsageReads, member_row, running_board,
    usage,
};
use crate::application::swarm::dto::{
    BudgetEffect, NewRequestUsage, RecordRequestUsageRequest, RequestDelivery,
};
use crate::application::swarm::ports::BoardEncoding;
use crate::domain::swarm::{BoardError, RefusalKind, RunState};

fn service(state: BoardState) -> (std::sync::Arc<MemoryBoard>, RecordRequestUsage) {
    let board = MemoryBoard::with(state);
    let service = RecordRequestUsage::new(
        board.clone(),
        SteppingClock::fixed(50.0),
        std::sync::Arc::new(CompactEncoding),
    );
    (board, service)
}

fn with_worker() -> BoardState {
    let mut state = running_board(100.0);
    state.members.push(member_row("worker", "live"));
    state
}

fn record(actor: &str, record: Value) -> RecordRequestUsageRequest {
    RecordRequestUsageRequest {
        actor: actor.to_owned(),
        record,
    }
}

/// The text the board's encoder writes for `record`: what the use case
/// bounds and stores.
fn encoded(record: &Value) -> String {
    CompactEncoding.encode(record).unwrap()
}

fn measured() -> Value {
    json!({"request_id": "r1", "context_input_tokens": 70, "input_tokens": 50,
           "output_tokens": 10, "cache_read_tokens": 20, "cache_write_tokens": 0,
           "instrumented_attempts": 2, "outcome": "succeeded"})
}

/// The record is measured before the operation gate.
#[test]
fn an_invalid_record_is_refused_before_the_store() {
    let (board, service) = service(with_worker());
    assert_eq!(
        service
            .execute(record("worker", json!({"request_id": "r"})))
            .unwrap_err(),
        BoardError::new(RefusalKind::Invalid, "invalid request observation")
    );
    assert!(board.transactions().is_empty());
}

/// A new record is inserted with its measurement and the four reported
/// counts copied from it (not the context input), and the answer is the
/// control receipt; a dead member may still record.
#[test]
fn a_new_record_is_inserted_with_its_measurement() {
    let mut state = with_worker();
    state.members.push(member_row("gone", "dead"));
    let (board, service) = service(state);
    let recorded = service.execute(record("gone", measured())).unwrap();
    assert_eq!(recorded.delivery, RequestDelivery::Recorded);
    assert_eq!(recorded.effect, BudgetEffect::Unchanged);
    assert_eq!(recorded.receipt.status.as_deref(), Some("running"));
    assert_eq!(
        board.snapshot().request_usage,
        [NewRequestUsage {
            request_id: "r1".to_owned(),
            actor: "gone".to_owned(),
            payload: encoded(&measured()),
            tokens: 80,
            unknown: 0,
            attempts: 2,
            input_tokens: Some(50),
            output_tokens: Some(10),
            cache_read_tokens: Some(20),
            cache_write_tokens: Some(0),
        }]
    );
}

/// The encoded record is bounded to 32,768 bytes.
#[test]
fn an_oversized_record_is_refused() {
    let (board, service) = service(with_worker());
    let mut huge = json!({"request_id": "huge", "instrumented_attempts": 1, "outcome": "failed"});
    huge["error_class"] = Value::from("x".repeat(32_768));
    assert_eq!(
        service.execute(record("worker", huge)).unwrap_err(),
        BoardError::new(
            RefusalKind::Invalid,
            "request diagnostic exceeds 32768 bytes"
        )
    );
    assert!(board.snapshot().request_usage.is_empty());
}

/// The same record from the same actor is a redelivery that writes
/// nothing; a digest becoming known replaces the stored record; another
/// actor or other data is refused.
#[test]
fn a_redelivery_is_accepted_only_as_the_same_record() {
    let (board, service) = service(with_worker());
    let pending = json!({"request_id": "d", "instrumented_attempts": 1, "outcome": "failed",
        "runtime": {"process_instance_id": "same", "executable_digest_pending": true}});
    service.execute(record("worker", pending.clone())).unwrap();
    let again = service.execute(record("worker", pending.clone())).unwrap();
    assert_eq!(again.delivery, RequestDelivery::Redelivered);
    let reused = BoardError::new(
        RefusalKind::RequestIdReused,
        "request observation ID reused with different data",
    );
    assert_eq!(
        service.execute(record("parent", pending)).unwrap_err(),
        reused
    );
    let known = json!({"request_id": "d", "instrumented_attempts": 1, "outcome": "failed",
        "runtime": {"process_instance_id": "same", "executable_digest_pending": false,
                    "executable_sha256": "abc"}});
    let digest = service.execute(record("worker", known.clone())).unwrap();
    assert_eq!(digest.delivery, RequestDelivery::Replaced);
    assert_eq!(board.snapshot().request_usage[0].payload, encoded(&known));
    let mut other = known;
    other["runtime"]["executable_sha256"] = json!("different");
    assert_eq!(
        service.execute(record("worker", other)).unwrap_err(),
        reused
    );
    assert_eq!(
        board.journal(),
        ["insert_request_usage d", "update_request_usage d"]
    );
}

/// The ledger holds 10,000 rows: the 10,000th is inserted, the next
/// refused, and the run keeps going.
#[test]
fn the_ledger_holds_ten_thousand_rows() {
    let with_rows = |rows: u64| {
        let mut state = with_worker();
        let mut report = usage(
            json!({"token_limit": null, "strict_unknown": false, "warned": false}),
            0,
            0,
        );
        report.totals.columns[0].1 = json!(rows);
        state.usage = Some(report);
        state
    };
    let failed = json!({"request_id": "overflow", "instrumented_attempts": 1, "outcome": "failed"});
    let (_, below) = service(with_rows(9_999));
    below.execute(record("worker", failed.clone())).unwrap();
    let (board, full) = service(with_rows(10_000));
    assert_eq!(
        full.execute(record("worker", failed)).unwrap_err(),
        BoardError::new(
            RefusalKind::CapacityFull,
            "request diagnostic ledger full; export before starting another run"
        )
    );
    assert!(board.journal().is_empty());
}

/// `test_usage_budget_is_idempotent_warns_once_and_pauses_without_losing_claims`:
/// crossing 80% warns once; reaching the limit pauses the running run.
#[test]
fn the_budget_warns_once_then_pauses() {
    let mut state = with_worker();
    state.usage = Some(usage(
        json!({"token_limit": 100, "strict_unknown": true, "warned": false}),
        0,
        0,
    ));
    let (board, service) = service(state);
    let first = service.execute(record("worker", measured())).unwrap();
    assert_eq!(first.effect, BudgetEffect::Warned);
    assert_eq!(
        first.receipt.budget.get("warned"),
        Some(&json!(true)),
        "the receipt reads the warning"
    );
    let mut second = measured();
    second["request_id"] = json!("r2");
    second["context_input_tokens"] = json!(20);
    second["output_tokens"] = json!(5);
    let paused = service.execute(record("worker", second)).unwrap();
    assert_eq!(paused.effect, BudgetEffect::Paused);
    assert_eq!(paused.receipt.outcome.as_deref(), Some("budget-exhausted"));
    let state = board.snapshot();
    assert_eq!(state.run.unwrap().record.status, Some(RunState::PAUSED));
    let actions: Vec<_> = state.events.iter().map(|e| e.action.as_str()).collect();
    assert_eq!(actions, ["usage-warning", "stop", "paused"]);
    assert_eq!(
        state.events[1].actor, "worker",
        "the recording member ends it"
    );
}

/// #2340: a recorded request reads the ledger once, for the budget's
/// standing, and never the whole usage report (its per-member aggregates
/// and latest observations), whose cost grows with the run; the receipt
/// reuses the totals the budget decided on, even when the budget warns
/// and pauses the run in between.
#[test]
fn a_recorded_request_reads_the_ledger_once_and_never_the_whole_report() {
    let mut state = with_worker();
    state.usage = Some(usage(
        json!({"token_limit": 100, "strict_unknown": true, "warned": false}),
        0,
        0,
    ));
    let (board, service) = service(state);
    let mut expected = UsageReads::default();
    for (request_id, tokens) in [("r1", 70), ("r2", 20)] {
        let mut observed = measured();
        observed["request_id"] = json!(request_id);
        observed["context_input_tokens"] = json!(tokens);
        service.execute(record("worker", observed)).unwrap();
        expected.standings += 1;
        assert_eq!(board.snapshot().usage_reads, expected, "{request_id}");
    }
    assert_eq!(
        board.snapshot().run.unwrap().record.status,
        Some(RunState::PAUSED),
        "the second request paused the run, and its receipt listed blockers"
    );
}
