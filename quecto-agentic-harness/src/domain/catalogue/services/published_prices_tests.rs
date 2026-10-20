//! #1855: the one published price table, by id prefix, and how a declared
//! cost's missing rates are filled.
use super::*;

type Rates = (f64, f64, f64, f64);

fn known(id: &str) -> Option<Rates> {
    published_cost(id).map(|c| (c.input, c.output, c.cache_read, c.cache_write))
}

#[test]
fn the_table_prices_by_prefix_case_insensitively() {
    assert_eq!(
        known("claude-sonnet-5-20260630"),
        Some((2.0, 10.0, 0.2, 2.5))
    );
    assert_eq!(known("Claude-Sonnet-5-5"), Some((2.0, 10.0, 0.1, 2.5)));
    assert_eq!(known("claude-sonnet-4-6"), Some((3.0, 15.0, 0.3, 3.75)));
    assert_eq!(
        known("claude-haiku-4-5-20251001"),
        Some((1.0, 5.0, 0.1, 1.25))
    );
    for unknown in ["gpt-4o", "claude-3-5-sonnet-20241022", "unknown-model", ""] {
        assert_eq!(known(unknown), None, "{unknown:?}");
    }
}

/// #1855 review L3: Opus 4.5 to 4.8 are $5/$25; the original Opus 4,
/// however its id is spelled, and Opus 4.1 are $15/$75.
#[test]
fn only_opus_4_5_and_later_take_the_lower_opus_4_tier() {
    for id in [
        "claude-opus-4-5",
        "claude-opus-4-5-20251101",
        "claude-opus-4-6",
        "claude-opus-4-7",
        "claude-opus-4-8",
    ] {
        assert_eq!(known(id), Some((5.0, 25.0, 0.5, 6.25)), "{id}");
    }
    for id in [
        "claude-opus-4-0",
        "claude-opus-4-20250514",
        "claude-opus-4@20250514",
        "claude-opus-4-1",
        "claude-opus-4-1-20250805",
    ] {
        assert_eq!(known(id), Some((15.0, 75.0, 1.5, 18.75)), "{id}");
    }
}

/// #1855 review M1: the GPT-5.6 tiers' published rates (Sol's is its
/// promotional price) and the GPT-6 tiers', read from their model pages.
#[test]
fn openai_tiers_carry_their_published_rates() {
    for (id, rates) in [
        ("gpt-5.6-sol", (4.0, 20.0, 0.4, 5.0)),
        ("gpt-5.6-terra", (2.0, 12.0, 0.2, 2.5)),
        ("gpt-5.6-luna", (0.2, 1.2, 0.02, 0.25)),
        ("gpt-6-astra", (10.0, 50.0, 1.0, 12.5)),
        ("gpt-6-sol", (2.0, 10.0, 0.2, 2.5)),
        ("gpt-6.1-sol", (2.0, 10.0, 0.1, 2.5)),
        ("gpt-6-luna", (0.1, 0.5, 0.01, 0.125)),
    ] {
        assert_eq!(known(id), Some(rates), "{id}");
    }
}

/// #1855 review M2: Grok cache hits are priced ($0.50, or $0.30 for
/// Grok 4.5); xAI lists no cache-write price, so writes bill as input.
#[test]
fn grok_cache_reads_are_priced() {
    assert_eq!(known("grok-4.7"), Some((2.0, 6.0, 0.5, 2.0)));
    assert_eq!(known("grok-4.6"), Some((2.0, 6.0, 0.5, 2.0)));
    assert_eq!(known("grok-4.5"), Some((2.0, 6.0, 0.3, 2.0)));
}

/// #1855 review M2: a declared rate wins; an undeclared one is the table's
/// for the id, else (for a cache rate) the input rate, never a free one.
#[test]
fn a_declared_costs_missing_rates_are_filled() {
    let cost = |c: ModelCost| (c.input, c.output, c.cache_read, c.cache_write);
    // Known id: the table fills what is missing.
    assert_eq!(
        cost(declared_cost(
            "claude-opus-4-6",
            Some(1.0),
            Some(2.0),
            None,
            None
        )),
        (1.0, 2.0, 0.5, 6.25)
    );
    // Unknown id: cache rates fall back to the declared input rate.
    assert_eq!(
        cost(declared_cost(
            "local-model",
            Some(1.0),
            Some(2.0),
            None,
            None
        )),
        (1.0, 2.0, 1.0, 1.0)
    );
    // Declared rates win, a declared zero included.
    assert_eq!(
        cost(declared_cost(
            "grok-4.7",
            Some(3.0),
            Some(9.0),
            Some(0.0),
            Some(0.0)
        )),
        (3.0, 9.0, 0.0, 0.0)
    );
    // Nothing declared, nothing known: no price.
    assert_eq!(
        cost(declared_cost("local-model", None, None, None, None)),
        (0.0, 0.0, 0.0, 0.0)
    );
    assert_eq!(cost_of_id("local-model"), ModelCost::default());
    assert_eq!(cost_of_id("grok-4.5").cache_read, 0.3);
}
