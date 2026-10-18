//! Differential scenarios (#2274, epic #2265): usage accounting and the
//! inference admission read on the Python board and the
//! Rust board, compared after every step by result, refusal text and
//! logical database dump (so every `request_usage` and `usage_budget`
//! payload byte). Ported from the deleted Python suite `tests/swarm_helpers_test.py`, plus the
//! loosely typed arguments Python accepts (epic P3). `summary` is S12's, so
//! the scenarios read the run with `_snapshot` and `_control_status`.
use quecto::domain::inference::events::request_observation::{RequestObservation, RuntimeIdentity};
use quecto::domain::inference::services::provider_error::ProviderErrorClass;
use quecto::domain::inference::value_objects::attempt_diagnostics::AttemptDiagnostics;
use serde_json::{Value, json};

use crate::swarm_board_diff_membership::{at, create, snapshot};
use crate::swarm_board_diff_runs::NOW;
use crate::swarm_board_diff_runs::swarm_board_diff::Outcome;
use crate::swarm_board_diff_runs::swarm_board_diff::scenario::{
    Step, run_golden, run_rust, sql, step_text,
};

/// A running run of five coordinated by `parent`, with `worker` live.
pub(crate) fn with_worker(more: impl IntoIterator<Item = Step>) -> Vec<Step> {
    let mut steps = vec![
        create(5),
        at(1.0, "parent", "_admit", json!(["worker", "res-w"])),
        at(
            2.0,
            "parent",
            "_activate",
            json!(["worker", "res-w", 11, "t", null]),
        ),
    ];
    steps.extend(more);
    steps
}

/// `member` records `record`.
pub(crate) fn request(offset: f64, member: &str, record: Value) -> Step {
    at(offset, member, "_record_request", json!([record]))
}

fn read(offset: f64, member: &str, method: &str) -> Step {
    at(offset, member, method, json!([]))
}

/// `test_request_redelivery_accepts_runtime_digest_becoming_available`:
/// the stored record is replaced when its digest becomes known; another
/// digest, another process, another actor or other data is refused.
#[test]
fn request_redelivery_accepts_runtime_digest_becoming_available() {
    let pending = json!({"request_id": "digest", "instrumented_attempts": 1, "outcome": "failed",
        "runtime": {"process_instance_id": "same", "executable_digest_pending": true}});
    let known = json!({"request_id": "digest", "instrumented_attempts": 1, "outcome": "failed",
        "runtime": {"process_instance_id": "same", "executable_digest_pending": false,
                    "executable_sha256": "abc"}});
    let mut other_digest = known.clone();
    other_digest["runtime"]["executable_sha256"] = json!("different");
    let mut other_process = known.clone();
    other_process["runtime"]["process_instance_id"] = json!("different");
    let mut other_data = known.clone();
    other_data["instrumented_attempts"] = json!(2);
    run_golden(&with_worker([
        request(3.0, "worker", pending.clone()),
        request(4.0, "worker", pending.clone()),
        request(5.0, "worker", known.clone()),
        read(6.0, "parent", "usage_report"),
        request(7.0, "worker", known.clone()),
        request(8.0, "worker", pending),
        request(9.0, "worker", other_digest),
        request(10.0, "worker", other_process),
        request(11.0, "worker", other_data),
        request(12.0, "parent", known),
        read(13.0, "parent", "usage_report"),
    ]));
}

/// `test_request_diagnostic_ledger_bounds_are_explicit`: a record over
/// 32,768 encoded bytes is refused, and so is the 10,001st row, without
/// pausing the run.
#[test]
fn request_diagnostic_ledger_bounds_are_explicit() {
    let failed =
        |id: &str| json!({"request_id": id, "instrumented_attempts": 1, "outcome": "failed"});
    let mut huge = failed("huge");
    huge["error_class"] = json!("x".repeat(32_768));
    let mut edge = failed("edge");
    // `{"error_class":"…","instrumented_attempts":1,"outcome":"failed","request_id":"edge"}`
    // is exactly 32,768 bytes: accepted.
    let fixed =
        r#"{"error_class":"","instrumented_attempts":1,"outcome":"failed","request_id":"edge"}"#;
    edge["error_class"] = json!("y".repeat(32_768 - fixed.len()));
    // One byte more: `é` is written as its six-byte escape.
    let mut over = failed("over");
    over["error_class"] = json!(format!("{}é", "z".repeat(32_768 - fixed.len() - 5)));
    run_golden(&with_worker([
        request(3.0, "worker", failed("first")),
        request(4.0, "worker", huge),
        request(5.0, "worker", edge),
        request(6.0, "worker", over),
        sql(
            "WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i+1 FROM n WHERE i<9996) \
             INSERT INTO request_usage SELECT 'fill-'||i,'worker','{}',0,0,1,NULL,NULL,NULL,NULL FROM n;",
        ),
        request(7.0, "worker", failed("last")),
        request(8.0, "worker", failed("overflow")),
        request(9.0, "worker", failed("first")),
        snapshot(10.0),
        read(11.0, "parent", "_control_status"),
    ]));
}

/// A `RequestObservation` serialised as `swarm_coordination.rs` records
/// it (`runtime` included) round-trips identically: the stored payload
/// bytes, the report that reads it back, and its redelivery.
#[test]
fn a_real_request_observation_round_trips_identically() {
    let observation = RequestObservation {
        started_unix_ms: Some(1_000),
        finished_unix_ms: Some(1_750),
        attempt_diagnostics: vec![AttemptDiagnostics {
            attempt_number: 1,
            wire_status: Some(429),
            elapsed_ms: 12,
            first_token_ms: Some(7),
            ..Default::default()
        }],
        request_id: "observed-1".into(),
        model: "claude-test".into(),
        provider: "anthropic".into(),
        outcome: "failed".into(),
        error_class: Some(ProviderErrorClass::RateLimit),
        input_tokens: Some(1_200),
        context_input_tokens: Some(1_500),
        output_tokens: Some(300),
        cache_read_tokens: Some(0),
        cache_write_tokens: None,
        estimated_cost_micro_usd: Some(4_500),
        estimated_context_tokens: 1_480,
        instrumented_attempts: 2,
        oauth_retries: 1,
        duration_ms: 750,
        first_token_ms: None,
        harness_prefix_sha256: "ab".repeat(32),
        harness_prefix_bytes: 4_096,
        harness_prefix_unchanged: Some(true),
        ended_empty_after_tools: false,
        input_prefix: None,
    };
    let mut value = serde_json::to_value(&observation).unwrap();
    // A runtime identity of the shape `runtime_identity::current()` writes,
    // its values fixed so the scenario (and so its golden, #2283) is the
    // same in every process and at every version.
    value["runtime"] = serde_json::to_value(RuntimeIdentity {
        process_instance_id: "0d7de168-9413-4e8a-8a92-87285baaa850".into(),
        executable_digest_pending: true,
        package_version: "0.107.189".into(),
        build_source_revision: Some("8b90db2ba".into()),
        build_dirty: Some(false),
        executable_sha256: None,
    })
    .unwrap();
    let current =
        serde_json::to_value(quecto::infrastructure::processes::runtime_identity::current())
            .unwrap();
    let keys = |value: &Value| {
        value
            .as_object()
            .map(|entries| entries.keys().cloned().collect::<Vec<_>>())
    };
    assert_eq!(
        keys(&value["runtime"]),
        keys(&current),
        "the fixed identity has the real one's shape"
    );
    let text = json!([value]).to_string();
    // Python's golden of this scenario set a budget first (retired with
    // budgets, owner decision 2026-10-14): the Rust board's answers are
    // asserted instead.
    let last = |more: Vec<Step>| match run_rust(&with_worker(more)) {
        Outcome::Ok(answer) => answer,
        other => panic!("{other:?}"),
    };
    let recorded = || step_text("worker", "_record_request", &text, NOW + 4.0);
    let again = || step_text("worker", "_record_request", &text, NOW + 5.0);
    let first = last(vec![recorded()]);
    let redelivered = last(vec![recorded(), again()]);
    assert_eq!(
        (&first["status"], &first["budget"]["observed_tokens"]),
        (&json!("running"), &json!(1_800))
    );
    assert_eq!(redelivered, first, "a redelivery changes nothing");
    let report = last(vec![
        recorded(),
        again(),
        read(6.0, "parent", "usage_report"),
    ]);
    assert_eq!(report["totals"]["requests"], json!(1), "{report}");
    assert_eq!(
        report["recent_requests"][0]["observation"], value,
        "the stored record reads back as written"
    );
    let admission = last(vec![
        recorded(),
        again(),
        read(7.0, "worker", "_request_admission"),
    ]);
    assert_eq!(admission["status"], json!("running"));
}
