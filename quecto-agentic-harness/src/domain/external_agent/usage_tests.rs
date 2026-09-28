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
#[should_panic(expected = "a cumulative cost never shrinks")]
fn a_shrinking_cumulative_total_is_an_invariant_breach() {
    let mut ledger = UsageLedger::default();
    ledger.record(&with_total(0.02));
    ledger.record(&with_total(0.01));
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
}
