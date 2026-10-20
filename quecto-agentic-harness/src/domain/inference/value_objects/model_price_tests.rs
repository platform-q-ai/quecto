use super::*;

fn cost(input: f64, output: f64, cache_read: f64, cache_write: f64) -> ModelCost {
    ModelCost {
        input,
        output,
        cache_read,
        cache_write,
        declared: true,
    }
}

fn usage(prompt: u32, completion: u32, read: Option<u32>, write: Option<u32>) -> UsageInfo {
    UsageInfo {
        prompt_tokens: prompt,
        completion_tokens: completion,
        cache_read_tokens: read,
        cache_write_tokens: write,
        context_tokens: None,
        cost: None,
    }
}

#[test]
fn a_catalogue_cost_is_its_price_in_micro_usd_per_million() {
    assert_eq!(
        ModelPrice::of(&cost(2.0, 10.0, 0.1, 2.5)),
        Some(ModelPrice {
            input: 2_000_000,
            output: 10_000_000,
            cache_read: 100_000,
            cache_write: 2_500_000,
        })
    );
    // Fractions of a micro-dollar round to the nearest.
    assert_eq!(
        ModelPrice::of(&cost(0.125, 0.0, 0.01, 0.0)).map(|p| (p.input, p.cache_read)),
        Some((125_000, 10_000))
    );
}

#[test]
fn a_cost_that_declares_no_price_or_an_invalid_rate_is_no_price() {
    assert_eq!(
        ModelPrice::of(&ModelCost::default()),
        None,
        "nobody stated it"
    );
    // #2635 review M1: a stated all-zero cost is a free model's price.
    assert_eq!(
        ModelPrice::of(&cost(0.0, 0.0, 0.0, 0.0)),
        Some(ModelPrice {
            input: 0,
            output: 0,
            cache_read: 0,
            cache_write: 0,
        })
    );
    for bad in [f64::NAN, f64::INFINITY, -1.0] {
        assert_eq!(ModelPrice::of(&cost(bad, 10.0, 0.0, 0.0)), None, "{bad}");
    }
}

#[test]
fn usage_is_costed_per_kind_of_token_in_exact_integers() {
    // Sonnet 4.x rates: $3 in, $15 out, $0.30 cache read, $3.75 cache write.
    let price = ModelPrice::of(&cost(3.0, 15.0, 0.3, 3.75)).unwrap();
    let cost = price.cost_of(&usage(1000, 500, Some(200), Some(100)));
    assert_eq!(cost.input_cost_micro_usd, 3000);
    assert_eq!(cost.output_cost_micro_usd, 7500);
    assert_eq!(cost.cache_read_cost_micro_usd, 60);
    assert_eq!(cost.cache_write_cost_micro_usd, 375);
    assert_eq!(cost.total_cost_micro_usd, 3000 + 7500 + 60 + 375);
    assert!((cost.total_cost_usd() - 0.010935).abs() < 1e-9);
}

/// #2441: Sonnet 5 at its standard $2/$10 (cache $0.20 read, $2.50 write).
#[test]
fn sonnet_5_usage_is_costed_at_two_and_ten() {
    let price = ModelPrice::of(&cost(2.0, 10.0, 0.2, 2.5)).unwrap();
    let cost = price.cost_of(&usage(1_000_000, 100_000, Some(1_000_000), Some(1_000_000)));
    assert_eq!(cost.input_cost_micro_usd, 2_000_000);
    assert_eq!(cost.output_cost_micro_usd, 1_000_000);
    assert_eq!(cost.cache_read_cost_micro_usd, 200_000);
    assert_eq!(cost.cache_write_cost_micro_usd, 2_500_000);
    assert_eq!(cost.total_cost_micro_usd, 5_700_000);
}

#[test]
fn absent_cache_counts_cost_nothing() {
    let price = ModelPrice::of(&cost(5.0, 25.0, 0.5, 6.25)).unwrap();
    let cost = price.cost_of(&usage(1_000_000, 100_000, None, None));
    assert_eq!(
        (
            cost.cache_read_cost_micro_usd,
            cost.cache_write_cost_micro_usd
        ),
        (0, 0)
    );
    assert_eq!(cost.total_cost_micro_usd, 5_000_000 + 2_500_000);
}

/// #1855 review L2: an absurd declared rate saturates, never wraps.
#[test]
fn an_absurd_rate_saturates_rather_than_wrapping() {
    let price = ModelPrice::of(&cost(1e30, 1e30, 1e30, 1e30)).unwrap();
    assert_eq!(price.input, u64::MAX, "the float cast saturates");
    let cost = price.cost_of(&usage(u32::MAX, u32::MAX, Some(u32::MAX), Some(u32::MAX)));
    assert_eq!(cost.input_cost_micro_usd, u64::MAX / 1_000_000);
    assert_eq!(cost.total_cost_micro_usd, (u64::MAX / 1_000_000) * 4);
}
