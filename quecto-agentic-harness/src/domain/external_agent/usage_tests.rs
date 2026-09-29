//! Usage accounting (#2285).

use super::*;
use crate::domain::external_agent::stream::ModelUsage;

fn completed() -> ResultEvent {
    ResultEvent {
        is_error: Some(false),
        terminal_reason: Some("completed".into()),
        stop_reason: Some("end_turn".into()),
        ..ResultEvent::default()
    }
}

fn with_total(total: f64) -> ResultEvent {
    ResultEvent {
        total_cost_usd: Some(total),
        ..completed()
    }
}

#[test]
fn the_ledger_charges_each_turn_the_delta_of_the_cumulative_total() {
    let mut ledger = UsageLedger::default();
    assert_eq!(ledger.record(&with_total(0.044864)).cost_micro_usd, 44_864);
    let second = ledger.record(&with_total(0.05306469999999999));
    assert_eq!(second.cost_micro_usd, 8_201);
    assert_eq!(second.total_cost_micro_usd, 53_065);
    assert_eq!(ledger.total_cost_micro_usd(), 53_065);
}

#[test]
fn a_result_without_a_usable_total_costs_nothing_and_keeps_the_total() {
    let mut ledger = UsageLedger::default();
    ledger.record(&with_total(0.01));
    let missing = ledger.record(&completed());
    assert_eq!(missing.cost_micro_usd, 0);
    assert_eq!(missing.total_cost_micro_usd, 10_000);
    let nan = ledger.record(&with_total(f64::NAN));
    assert_eq!(nan.cost_micro_usd, 0);
    assert_eq!(ledger.total_cost_micro_usd(), 10_000);
}

#[test]
fn a_zero_total_inside_a_process_charges_nothing_and_never_double_charges() {
    // claude reports total_cost_usd 0 after a session crash, a cloud
    // delivery error or a bridge interrupt, inside the same process.
    let mut ledger = UsageLedger::default();
    assert_eq!(ledger.record(&with_total(0.053065)).cost_micro_usd, 53_065);
    let zero = ledger.record(&with_total(0.0));
    assert_eq!(zero.cost_micro_usd, 0);
    assert_eq!(
        zero.total_cost_micro_usd, 53_065,
        "the session total never drops"
    );
    assert_eq!(
        zero.cost_drop,
        Some(CostDrop {
            previous_micro_usd: 53_065,
            reported_micro_usd: 0,
        })
    );
    let next = ledger.record(&with_total(0.06));
    assert_eq!(next.cost_micro_usd, 6_935);
    assert_eq!(next.total_cost_micro_usd, 60_000, "the true total, 0.06");
    assert_eq!(next.cost_drop, None);
}

#[test]
fn a_new_process_is_charged_from_zero() {
    let mut ledger = UsageLedger::default();
    ledger.record(&with_total(0.053065));
    ledger.process_started();
    let first = ledger.record(&with_total(0.06));
    assert_eq!(
        first.cost_micro_usd, 60_000,
        "a higher first total is not undercounted"
    );
    assert_eq!(
        first.total_cost_micro_usd, 113_065,
        "the session sums its charges"
    );
    assert_eq!(first.cost_drop, None);
    ledger.process_started();
    let lower = ledger.record(&with_total(0.005));
    assert_eq!(lower.cost_micro_usd, 5_000);
    assert_eq!(lower.cost_drop, None);
    assert_eq!(ledger.total_cost_micro_usd(), 118_065);
}

#[test]
fn session_tokens_sum_each_process_cumulative_usage() {
    let with_model_usage = |input: u64| ResultEvent {
        model_usage: vec![ModelUsage {
            model: "claude-haiku-4-5-20251001".into(),
            tokens: TokenCounts {
                input,
                ..TokenCounts::default()
            },
            cost_usd: None,
        }],
        ..completed()
    };
    let mut ledger = UsageLedger::default();
    ledger.record(&with_model_usage(10));
    ledger.record(&with_model_usage(25));
    ledger.process_started();
    ledger.record(&with_model_usage(7));
    assert_eq!(ledger.cumulative_tokens().input, 32);
}

#[test]
fn turn_tokens_are_the_results_own_and_cumulative_tokens_are_model_usage() {
    let turn = TokenCounts {
        input: 26,
        output: 362,
        cache_read: 46_907,
        cache_write: 837,
    };
    let cumulative = TokenCounts {
        input: 84,
        output: 1431,
        cache_read: 133_457,
        cache_write: 16_240,
    };
    let result = ResultEvent {
        usage: turn,
        model_usage: vec![ModelUsage {
            model: "claude-haiku-4-5-20251001".into(),
            tokens: cumulative,
            cost_usd: Some(0.0530647),
        }],
        ..completed()
    };
    let mut ledger = UsageLedger::default();
    assert_eq!(ledger.record(&result).tokens, turn);
    assert_eq!(ledger.cumulative_tokens(), cumulative);
}

#[test]
fn without_model_usage_the_cumulative_tokens_sum_the_turns() {
    let turn = TokenCounts {
        input: 1,
        output: 2,
        cache_read: 3,
        cache_write: 4,
    };
    let result = ResultEvent {
        usage: turn,
        ..completed()
    };
    let mut ledger = UsageLedger::default();
    ledger.record(&result);
    ledger.record(&result);
    assert_eq!(ledger.cumulative_tokens(), turn.plus(turn));
}

#[test]
fn micro_usd_rounds_once_and_refuses_unusable_amounts() {
    assert_eq!(micro_usd(0.044864), Some(44_864));
    assert_eq!(micro_usd(0.0000004), Some(0));
    assert_eq!(micro_usd(0.0000005), Some(1));
    assert_eq!(micro_usd(-0.01), None);
    assert_eq!(micro_usd(f64::INFINITY), None);
    assert_eq!(micro_usd(f64::NAN), None);
    // Past 2^64 micro-USD does not fit a u64 (and `u64::MAX as f64` is 2^64).
    assert_eq!(micro_usd(1.9e13), None);
    assert_eq!(micro_usd(1e12), Some(1_000_000_000_000_000_000));
}
