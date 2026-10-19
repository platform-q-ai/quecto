//! #1855, #2441, #2328: the catalogue's model cost is the one price
//! authority. Each built-in carries its published standard rates (USD per
//! million tokens; cache writes are the 5-minute TTL), and usage is never
//! priced from any other table.
use super::*;

/// `(input, output, cache read, cache write)` USD per million tokens.
fn rates(cost: &ModelCost) -> (f64, f64, f64, f64) {
    (cost.input, cost.output, cost.cache_read, cost.cache_write)
}

/// Anthropic's published standard rates (platform.claude.com/docs/en/
/// about-claude/pricing, read 2026-10-19). Sonnet 5's $2/$10 introductory
/// price is now its standard one (footnote 3), and Opus 5.5 and Sonnet 5.5
/// cache hits are 0.05x input (footnote 2).
const ANTHROPIC_RATES: &[(&str, (f64, f64, f64, f64))] = &[
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
