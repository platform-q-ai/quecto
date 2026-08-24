use super::*;
use crate::domain::message::CostInfo;

#[test]
fn usage_totals_record_accumulates_billed_usage_and_keeps_latest_context() {
    let mut totals = UsageTotals::default();
    totals.record(&UsageInfo {
        prompt_tokens: 10,
        completion_tokens: 2,
        cache_read_tokens: Some(3),
        cache_write_tokens: Some(4),
        context_tokens: None,
        cost: Some(CostInfo {
            input_cost_micro_usd: 0,
            output_cost_micro_usd: 0,
            cache_read_cost_micro_usd: 0,
            cache_write_cost_micro_usd: 0,
            total_cost_micro_usd: 1_000,
        }),
    });
    totals.record(&UsageInfo {
        prompt_tokens: 20,
        completion_tokens: 5,
        cache_read_tokens: None,
        cache_write_tokens: None,
        context_tokens: None,
        cost: None,
    });

    assert_eq!(totals.context_input_tokens, 20);
    assert_eq!(totals.output_tokens, 7);
    assert_eq!(totals.billed_input_tokens, 30);
    assert_eq!(totals.billed_output_tokens, 7);
    assert_eq!(totals.cache_read_tokens, 3);
    assert_eq!(totals.cache_write_tokens, 4);
    assert_eq!(totals.cost_micro_usd, 1_000);
}

#[test]
fn record_uses_normalized_context_tokens_over_prompt_tokens() {
    // Anthropic-style: prompt_tokens is the non-cached delta, but
    // context_tokens carries the true occupancy (delta + cache). The
    // context gauge must reflect the latter while billing stays on
    // prompt_tokens.
    let mut totals = UsageTotals::default();
    totals.record(&UsageInfo {
        prompt_tokens: 100,
        completion_tokens: 5,
        cache_read_tokens: Some(80),
        cache_write_tokens: Some(20),
        context_tokens: Some(200),
        cost: None,
    });
    assert_eq!(
        totals.context_input_tokens, 200,
        "gauge uses context_tokens"
    );
    assert_eq!(
        totals.billed_input_tokens, 100,
        "billing stays on prompt_tokens"
    );
}

#[test]
fn record_falls_back_to_prompt_tokens_when_context_tokens_absent() {
    // Compatibility fallback: providers without explicit context occupancy use
    // prompt_tokens as the context gauge.
    let mut totals = UsageTotals::default();
    totals.record(&UsageInfo {
        prompt_tokens: 150,
        completion_tokens: 5,
        cache_read_tokens: None,
        cache_write_tokens: None,
        context_tokens: None,
        cost: None,
    });
    assert_eq!(totals.context_input_tokens, 150);
}

fn assert_ratio(actual: Option<f64>, expected: f64) {
    let actual = actual.expect("ratio should be present");
    assert!(
        (actual - expected).abs() < 1e-9,
        "expected {expected}, got {actual}"
    );
}

#[test]
fn usage_totals_cache_hit_ratio_none_without_denominator() {
    assert_eq!(
        crate::domain::usage_accounting::cache_hit_ratio(0, 0, 0),
        None
    );
}

#[test]
fn usage_totals_cache_hit_ratio_mixed_input_and_read() {
    assert_ratio(
        crate::domain::usage_accounting::cache_hit_ratio(70, 30, 0),
        0.30,
    );
}

#[test]
fn usage_totals_cache_hit_ratio_write_only_zero_hit() {
    assert_ratio(
        crate::domain::usage_accounting::cache_hit_ratio(0, 0, 50),
        0.0,
    );
}

#[test]
fn usage_totals_cache_hit_ratio_read_only_full_hit() {
    assert_ratio(
        crate::domain::usage_accounting::cache_hit_ratio(0, 30, 0),
        1.0,
    );
}

#[test]
fn usage_totals_cache_hit_ratio_read_and_write() {
    assert_ratio(
        crate::domain::usage_accounting::cache_hit_ratio(0, 30, 20),
        30.0 / 50.0,
    );
}

#[test]
fn usage_totals_cache_hit_ratio_uncached_input_zero_hit() {
    assert_ratio(
        crate::domain::usage_accounting::cache_hit_ratio(70, 0, 0),
        0.0,
    );
}

#[test]
fn session_stats_from_shared_usage_fixture_reports_cost_and_ratio() {
    let mut totals = UsageTotals::default();
    totals.record(&UsageInfo {
        prompt_tokens: 70,
        completion_tokens: 20,
        cache_read_tokens: Some(30),
        cache_write_tokens: Some(5),
        context_tokens: Some(105),
        cost: Some(CostInfo {
            input_cost_micro_usd: 0,
            output_cost_micro_usd: 0,
            cache_read_cost_micro_usd: 0,
            cache_write_cost_micro_usd: 0,
            total_cost_micro_usd: 1_234,
        }),
    });

    assert_eq!(totals.billed_input_tokens, 70);
    assert_eq!(totals.billed_output_tokens, 20);
    assert_eq!(totals.cache_read_tokens, 30);
    assert_eq!(totals.cache_write_tokens, 5);
    assert_eq!(totals.context_input_tokens, 105);
    assert_eq!(totals.cost_micro_usd, 1_234);
    assert_ratio(
        crate::domain::usage_accounting::cache_hit_ratio(
            totals.billed_input_tokens,
            totals.cache_read_tokens,
            totals.cache_write_tokens,
        ),
        30.0 / 105.0,
    );
}

#[test]
fn equivalent_provider_usage_produces_identical_shared_stats() {
    let equivalent_usages = [
        UsageInfo {
            // OpenAI/Codex equivalent after adapter normalization.
            prompt_tokens: 70,
            completion_tokens: 20,
            cache_read_tokens: Some(30),
            cache_write_tokens: Some(5),
            context_tokens: Some(105),
            cost: Some(CostInfo {
                input_cost_micro_usd: 0,
                output_cost_micro_usd: 0,
                cache_read_cost_micro_usd: 0,
                cache_write_cost_micro_usd: 0,
                total_cost_micro_usd: 1_234,
            }),
        },
        UsageInfo {
            // Anthropic equivalent normalized into the same UsageInfo contract.
            prompt_tokens: 70,
            completion_tokens: 20,
            cache_read_tokens: Some(30),
            cache_write_tokens: Some(5),
            context_tokens: Some(105),
            cost: Some(CostInfo {
                input_cost_micro_usd: 0,
                output_cost_micro_usd: 0,
                cache_read_cost_micro_usd: 0,
                cache_write_cost_micro_usd: 0,
                total_cost_micro_usd: 1_234,
            }),
        },
    ];

    let snapshots: Vec<_> = equivalent_usages
        .iter()
        .map(|usage| {
            let mut totals = UsageTotals::default();
            totals.record(usage);
            (
                totals.billed_input_tokens,
                totals.billed_output_tokens,
                totals.cache_read_tokens,
                totals.cache_write_tokens,
                totals.context_input_tokens,
                totals.cost_micro_usd,
                crate::domain::usage_accounting::cache_hit_ratio(
                    totals.billed_input_tokens,
                    totals.cache_read_tokens,
                    totals.cache_write_tokens,
                ),
            )
        })
        .collect();

    assert_eq!(snapshots[0], snapshots[1]);
    assert_ratio(snapshots[0].6, 30.0 / 105.0);
}

#[test]
fn usage_totals_record_saturates_counters() {
    let mut totals = UsageTotals {
        billed_input_tokens: u64::MAX,
        billed_output_tokens: u64::MAX,
        cache_read_tokens: u64::MAX,
        cache_write_tokens: u64::MAX,
        cost_micro_usd: u64::MAX,
        output_tokens: u32::MAX,
        context_input_tokens: 0,
    };
    totals.record(&UsageInfo {
        prompt_tokens: 1,
        completion_tokens: 1,
        cache_read_tokens: Some(1),
        cache_write_tokens: Some(1),
        context_tokens: None,
        cost: Some(CostInfo {
            input_cost_micro_usd: 0,
            output_cost_micro_usd: 0,
            cache_read_cost_micro_usd: 0,
            cache_write_cost_micro_usd: 0,
            total_cost_micro_usd: 1,
        }),
    });

    assert_eq!(totals.output_tokens, u32::MAX);
    assert_eq!(totals.billed_input_tokens, u64::MAX);
    assert_eq!(totals.billed_output_tokens, u64::MAX);
    assert_eq!(totals.cache_read_tokens, u64::MAX);
    assert_eq!(totals.cache_write_tokens, u64::MAX);
    assert_eq!(totals.cost_micro_usd, u64::MAX);
}
