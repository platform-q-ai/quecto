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

/// #1855 review M2: a `models.json` model the table knows is priced from
/// it; a declared cost drives billing, and a rate it leaves out is the
/// table's for the id (else, for a cache rate, the declared input rate),
/// never a free one.
#[test]
fn a_declared_cost_wins_and_its_missing_rates_are_never_free() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("models.json");
    std::fs::write(
        &path,
        r#"{"providers":{"anthropic-api":{"api":"anthropic-messages","models":[
            {"id":"claude-sonnet-4-6"},
            {"id":"claude-opus-4-6","cost":{"input":1.0,"output":2.0}},
            {"id":"local-model","cost":{"input":1.0,"output":2.0}},
            {"id":"free-model","cost":{"input":0.0,"output":0.0,"cacheRead":0.0,"cacheWrite":0.0}}
        ]}}}"#,
    )
    .unwrap();
    let registry = ModelRegistry::load_from_path(&path).unwrap();
    let cost = |id: &str| rates(&registry.find("anthropic-api", id).unwrap().cost);
    assert_eq!(cost("claude-sonnet-4-6"), (3.0, 15.0, 0.3, 3.75));
    assert_eq!(cost("claude-opus-4-6"), (1.0, 2.0, 0.5, 6.25));
    assert_eq!(cost("local-model"), (1.0, 2.0, 1.0, 1.0));
    assert_eq!(cost("free-model"), (0.0, 0.0, 0.0, 0.0));
}

/// Every built-in Grok is priced from the table too (#1855: before, its
/// usage went uncosted while the catalogue listed a price).
#[test]
fn every_grok_built_in_is_priced() {
    let registry = ModelRegistry::builtin();
    for (id, expected) in [
        ("grok-4.7", (2.0, 6.0, 0.5, 2.0)),
        ("grok-4.6", (2.0, 6.0, 0.5, 2.0)),
        ("grok-4.5", (2.0, 6.0, 0.3, 2.0)),
    ] {
        let record = registry.find("xai", id).expect("built in");
        assert_eq!(rates(&record.cost), expected, "{id}");
    }
}

/// #2630 review L2: every built-in model has a price row.
#[test]
fn every_built_in_has_a_price() {
    use crate::domain::inference::value_objects::model_price::ModelPrice;
    for record in ModelRegistry::builtin().models() {
        let cost = crate::domain::catalogue::value_objects::model_facts::ModelCost {
            input: record.cost.input,
            output: record.cost.output,
            cache_read: record.cost.cache_read,
            cache_write: record.cost.cache_write,
            declared: record.cost.declared,
        };
        assert!(ModelPrice::of(&cost).is_some(), "{}", record.qualified_id());
    }
}

/// Guards the id-string coupling between `builtin_specs()` and
/// `gpt_5_6_tier`: the enrichment in `build_builtin` is keyed by literal id
/// match, so a tier added or renamed in one table but not the other would
/// otherwise silently ship with default limits and `reasoning = false`; its
/// price is the one table's (`domain::catalogue::services::published_prices`, #1855).
/// This walks every built-in `gpt-5.6*` id rather than a
/// hardcoded list, failing loudly on drift.
#[test]
fn every_builtin_gpt_5_6_id_has_pricing_and_enriched_limits() {
    let registry = ModelRegistry::builtin();
    let tiers: Vec<_> = registry
        .models()
        .iter()
        .filter(|m| m.id.starts_with("gpt-5.6"))
        .collect();
    assert!(!tiers.is_empty(), "expected built-in gpt-5.6 tiers");
    for m in tiers {
        let id = &m.id;
        assert!(
            super::gpt_5_6_tier(id),
            "{id} is in builtin_specs() but gpt_5_6_tier has no entry for it; \
             add it there so the model does not ship with default limits"
        );
        assert!(m.reasoning, "{id} enrichment not applied: reasoning");
        assert!(
            m.context_window_explicit,
            "{id} context window not explicit"
        );
        assert!(m.max_tokens_explicit, "{id} max tokens not explicit");
        assert!(m.cost.input > 0.0, "{id} input price missing");
        assert!(m.cost.output > 0.0, "{id} output price missing");
    }
}

/// #2630 review round 4: a declared rate out of range (negative, or not
/// finite) is reported and the declaration ignored: the published rate for
/// a known id applies, and an unknown id stays unpriced, never silently.
#[test]
fn an_out_of_range_declared_rate_is_reported_and_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("models.json");
    std::fs::write(
        &path,
        r#"{"providers":{"local":{"api":"openai-completions","baseUrl":"http://127.0.0.1:8080/v1","models":[
            {"id":"claude-sonnet-4-6","cost":{"input":-1.0,"output":2.0}},
            {"id":"my-model","cost":{"input":1.0,"output":-2.0}},
            {"id":"good-model","cost":{"input":1.0,"output":2.0}}
        ]}}}"#,
    )
    .unwrap();
    let config = ModelRegistry::load_registry_config(&path).unwrap();
    let reasons: Vec<&str> = config
        .skipped
        .iter()
        .map(|block| block.error.as_str())
        .collect();
    assert_eq!(reasons.len(), 2, "{reasons:?}");
    assert!(
        reasons[0].contains("'claude-sonnet-4-6' declares a cost rate out of range"),
        "{reasons:?}"
    );
    let record = |id: &str| {
        config
            .records
            .iter()
            .find(|r| r.id == id)
            .expect(id)
            .cost
            .clone()
    };
    assert_eq!(
        rates(&record("claude-sonnet-4-6")),
        (3.0, 15.0, 0.3, 3.75),
        "published"
    );
    assert!(!record("my-model").declared, "unpriced");
    assert_eq!(rates(&record("good-model")), (1.0, 2.0, 1.0, 1.0));
}
