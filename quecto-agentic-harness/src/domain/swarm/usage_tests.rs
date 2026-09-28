//! `usage_budget_decision` and `request_measurement`, messages asserted by string.
use serde_json::{Value, json};

use super::*;

fn budget(token_limit: Option<u64>, strict_unknown: bool) -> UsageBudget {
    UsageBudget {
        token_limit,
        strict_unknown,
        warned: false,
    }
}

fn totals(observed_tokens: u64, unknown_usage_requests: u64) -> UsageTotals {
    UsageTotals {
        observed_tokens,
        unknown_usage_requests,
    }
}

fn refusal(record: Value) -> String {
    request_measurement(&record)
        .expect_err("the measurement is refused")
        .to_string()
}

fn record() -> Value {
    json!({
        "request_id": "r1",
        "input_tokens": 10,
        "context_input_tokens": 100,
        "output_tokens": 20,
        "cache_read_tokens": null,
        "instrumented_attempts": 1,
        "outcome": "succeeded",
    })
}

fn with(key: &str, value: Value) -> Value {
    let mut record = record();
    record[key] = value;
    record
}

#[test]
fn usage_decision_table() {
    use UsageDecision::{Allow, Pause, Warn};
    assert_eq!(
        usage_budget_decision(&budget(None, true), &totals(u64::MAX, 9)),
        Allow
    );
    assert_eq!(
        usage_budget_decision(&budget(Some(100), true), &totals(0, 1)),
        Pause
    );
    assert_eq!(
        usage_budget_decision(&budget(Some(100), false), &totals(0, 1)),
        Allow
    );
    assert_eq!(
        usage_budget_decision(&budget(Some(100), false), &totals(100, 0)),
        Pause
    );
    assert_eq!(
        usage_budget_decision(&budget(Some(100), false), &totals(99, 0)),
        Warn
    );
    assert_eq!(
        usage_budget_decision(&budget(Some(100), false), &totals(80, 0)),
        Warn
    );
    assert_eq!(
        usage_budget_decision(&budget(Some(100), false), &totals(79, 0)),
        Allow
    );
    // Integer math: 4 * 5 >= 5 * 4 warns where a float 0.8 might not.
    assert_eq!(
        usage_budget_decision(&budget(Some(5), false), &totals(4, 0)),
        Warn
    );
    assert_eq!(
        usage_budget_decision(&budget(Some(6), false), &totals(4, 0)),
        Allow
    );
    assert_eq!(
        usage_budget_decision(&budget(Some(0), false), &totals(0, 0)),
        Pause
    );
    // Python integers never overflow: neither does the warning threshold.
    let huge = u64::MAX;
    assert_eq!(
        usage_budget_decision(&budget(Some(huge), false), &totals(huge - 1, 0)),
        Warn
    );
    assert_eq!(
        usage_budget_decision(&budget(Some(huge), false), &totals(huge / 2, 0)),
        Allow
    );
    assert_eq!(
        [Allow, Warn, Pause].map(UsageDecision::as_str),
        ["allow", "warn", "pause"]
    );
}

#[test]
fn request_measurement_sums_context_and_output_tokens() {
    assert_eq!(request_measurement(&record()), Ok((120, 0, 1)));
    let most = (1u64 << 32) - 1;
    let full = with("output_tokens", json!(most));
    assert_eq!(request_measurement(&full), Ok((100 + most, 0, 1)));
    let bare = json!({"request_id": "r", "instrumented_attempts": 0, "outcome": "failed"});
    assert_eq!(request_measurement(&bare), Ok((0, 0, 0)));
}

#[test]
fn request_measurement_rejects_each_invalid_field() {
    let too_big = json!(1u64 << 32);
    for field in [
        "input_tokens",
        "context_input_tokens",
        "output_tokens",
        "cache_read_tokens",
        "cache_write_tokens",
    ] {
        let expected = format!("invalid request usage {field}");
        assert_eq!(refusal(with(field, too_big.clone())), expected);
        assert_eq!(refusal(with(field, json!(true))), expected);
        assert_eq!(refusal(with(field, json!(-1))), expected);
        assert_eq!(refusal(with(field, json!(5.0))), expected);
        assert_eq!(refusal(with(field, json!("5"))), expected);
    }
    let observation = "invalid request observation";
    assert_eq!(refusal(with("outcome", json!("timeout"))), observation);
    assert_eq!(refusal(with("outcome", json!(null))), observation);
    assert_eq!(refusal(with("instrumented_attempts", too_big)), observation);
    assert_eq!(
        refusal(with("instrumented_attempts", json!(true))),
        observation
    );
    assert_eq!(
        refusal(with("instrumented_attempts", json!(null))),
        observation
    );
    assert_eq!(refusal(with("request_id", json!(""))), observation);
    assert_eq!(
        refusal(with("request_id", json!("x".repeat(129)))),
        observation
    );
    assert_eq!(refusal(with("request_id", json!(7))), observation);
    assert_eq!(refusal(json!([record()])), observation);
    assert_eq!(refusal(json!(null)), observation);
    let mut missing = record();
    missing
        .as_object_mut()
        .expect("an object")
        .remove("request_id");
    assert_eq!(refusal(missing), observation);
    // 128 characters, counted as Python counts them (code points, not bytes).
    let limit = with("request_id", json!("é".repeat(128)));
    assert_eq!(request_measurement(&limit), Ok((120, 0, 1)));
    // A bad request id is judged before a bad token field.
    let both = with("request_id", json!(""));
    let mut both = both;
    both["input_tokens"] = json!(-1);
    assert_eq!(refusal(both), observation);
}

#[test]
fn request_measurement_counts_unknown_only_for_answered_attempts() {
    let unmeasured = |outcome: &str, attempts: u64| {
        let mut record = with("outcome", json!(outcome));
        record["instrumented_attempts"] = json!(attempts);
        record["output_tokens"] = Value::Null;
        request_measurement(&record)
    };
    assert_eq!(unmeasured("succeeded", 2), Ok((0, 1, 2)));
    assert_eq!(unmeasured("failed", 1), Ok((0, 1, 1)));
    assert_eq!(unmeasured("cancelled", 1), Ok((0, 0, 1)));
    assert_eq!(unmeasured("rejected", 3), Ok((0, 0, 3)));
    assert_eq!(unmeasured("succeeded", 0), Ok((0, 0, 0)));
    // A missing context count is unknown too.
    let mut missing = record();
    missing
        .as_object_mut()
        .expect("an object")
        .remove("context_input_tokens");
    assert_eq!(request_measurement(&missing), Ok((0, 1, 1)));
}
