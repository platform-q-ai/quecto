//! #1855, #2441, #2328: the catalogue's model cost is the one price
//! authority. Each built-in carries its published standard rates (USD per
//! million tokens; cache writes are the 5-minute TTL), and usage is never
//! priced from any other table.
use super::*;

/// `(input, output, cache read, cache write)` USD per million tokens.
type Rates = (f64, f64, f64, f64);

fn rates(cost: &ModelCost) -> Rates {
    (cost.input, cost.output, cost.cache_read, cost.cache_write)
}

/// Anthropic's published standard rates (platform.claude.com/docs/en/
/// about-claude/pricing, read 2026-10-19). Sonnet 5's $2/$10 introductory
/// price is now its standard one (footnote 3), and Opus 5.5 and Sonnet 5.5
/// cache hits are 0.05x input (footnote 2).
const ANTHROPIC_RATES: &[(&str, Rates)] = &[
    ("claude-fable-5-1", (10.0, 50.0, 0.25, 12.5)),
    ("claude-fable-5", (10.0, 50.0, 1.0, 12.5)),
    ("claude-opus-5-5", (4.0, 20.0, 0.2, 5.0)),
    ("claude-opus-5", (5.0, 25.0, 0.5, 6.25)),
    ("claude-sonnet-5-5", (2.0, 10.0, 0.1, 2.5)),
    ("claude-sonnet-5", (2.0, 10.0, 0.2, 2.5)),
];

#[test]
fn every_claude_built_in_carries_its_published_rates() {
    let registry = ModelRegistry::builtin();
    for provider in ["anthropic-api", "anthropic-oauth"] {
        for (id, expected) in ANTHROPIC_RATES {
            let record = registry.find(provider, id).expect("built in");
            assert_eq!(rates(&record.cost), *expected, "{provider}/{id}");
        }
    }
}

/// #2328: GPT-6.1 Sol's published standard rates
/// (developers.openai.com/api/docs/models/gpt-6.1-sol): $2 in, $0.10
/// cached, $10 out, $2.50 cache writes.
#[test]
fn gpt_6_1_sol_carries_its_published_rates() {
    let registry = ModelRegistry::builtin();
    for provider in ["openai-api", "openai-oauth"] {
        let record = registry.find(provider, "gpt-6.1-sol").expect("built in");
        assert_eq!(rates(&record.cost), (2.0, 10.0, 0.1, 2.5), "{provider}");
    }
}

/// The one table prices a model by case-insensitive id prefix, dated
/// variants included, and knows only its allowlist.
#[test]
fn the_table_prices_by_prefix_and_knows_only_its_allowlist() {
    let known = |id: &str| prices::known_cost(id).map(|cost| rates(&cost));
    assert_eq!(
        known("claude-sonnet-5-20260630"),
        Some((2.0, 10.0, 0.2, 2.5))
    );
    assert_eq!(known("Claude-Sonnet-5-5"), Some((2.0, 10.0, 0.1, 2.5)));
    assert_eq!(known("claude-opus-4-6"), Some((5.0, 25.0, 0.5, 6.25)));
    assert_eq!(known("claude-sonnet-4-6"), Some((3.0, 15.0, 0.3, 3.75)));
    assert_eq!(
        known("claude-haiku-4-5-20251001"),
        Some((1.0, 5.0, 0.1, 1.25))
    );
    // Opus 4.1 and the original Opus 4 kept the $15/$75 tier.
    assert_eq!(known("claude-opus-4-1"), Some((15.0, 75.0, 1.5, 18.75)));
    assert_eq!(
        known("claude-opus-4-20250514"),
        Some((15.0, 75.0, 1.5, 18.75))
    );
    for unknown in ["gpt-4o", "claude-3-5-sonnet-20241022", "unknown-model", ""] {
        assert_eq!(known(unknown), None, "{unknown:?}");
    }
}

/// A `models.json` model the table knows is priced from it unless its
/// declaration states a cost, which wins.
#[test]
fn a_declared_model_is_priced_from_the_table_unless_it_declares_a_cost() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("models.json");
    std::fs::write(
        &path,
        r#"{"providers":{"anthropic-api":{"api":"anthropic-messages","models":[
            {"id":"claude-sonnet-4-6"},
            {"id":"claude-opus-4-6","cost":{"input":1.0,"output":2.0}}
        ]}}}"#,
    )
    .unwrap();
    let registry = ModelRegistry::load_from_path(&path).unwrap();
    let sonnet = registry.find("anthropic-api", "claude-sonnet-4-6").unwrap();
    assert_eq!(rates(&sonnet.cost), (3.0, 15.0, 0.3, 3.75));
    let opus = registry.find("anthropic-api", "claude-opus-4-6").unwrap();
    assert_eq!(rates(&opus.cost), (1.0, 2.0, 0.0, 0.0));
}

/// Every built-in Grok is priced from the table too (#1855: before, its
/// usage went uncosted while the catalogue listed a price).
#[test]
fn every_grok_built_in_is_priced() {
    let registry = ModelRegistry::builtin();
    for (id, expected) in [
        ("grok-4.7", (2.0, 6.0, 0.0, 0.0)),
        ("grok-4.6", (2.0, 6.0, 0.5, 0.0)),
        ("grok-4.5", (2.0, 6.0, 0.0, 0.0)),
    ] {
        let record = registry.find("xai", id).expect("built in");
        assert_eq!(rates(&record.cost), expected, "{id}");
    }
}
