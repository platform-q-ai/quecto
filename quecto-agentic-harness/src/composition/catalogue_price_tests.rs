//! #1855 review M3: a model is priced at its published rates whatever path
//! it takes through the real catalogue to `ChangeActiveModel` — a dated id
//! the catalogue does not list, a retired model nothing declares, and a
//! discovered entry that replaces a built-in.
use crate::composition::catalogue::published_model_limits_for;
use crate::domain::inference::value_objects::model_price::ModelPrice;
use crate::infrastructure::providers::catalogue::catalogue_discovery::{
    DiscoverySourceCache, discovery_cache_dir,
};

fn micro(input: f64, output: f64, cache_read: f64, cache_write: f64) -> Option<ModelPrice> {
    let rate = |usd: f64| (usd * 1_000_000.0).round() as u64;
    Some(ModelPrice {
        input: rate(input),
        output: rate(output),
        cache_read: rate(cache_read),
        cache_write: rate(cache_write),
    })
}

#[test]
fn a_dated_id_the_catalogue_does_not_list_is_priced() {
    let tmp = tempfile::tempdir().unwrap();
    let limits = published_model_limits_for(tmp.path(), "anthropic-api/claude-sonnet-5-20260630");
    assert_eq!(limits.price, micro(2.0, 10.0, 0.2, 2.5));
}

#[test]
fn a_retired_model_nothing_declares_is_priced() {
    let tmp = tempfile::tempdir().unwrap();
    let limits = published_model_limits_for(tmp.path(), "anthropic-api/claude-sonnet-4-6");
    assert_eq!(limits.price, micro(3.0, 15.0, 0.3, 3.75));
}

#[test]
fn a_discovered_entry_replacing_a_built_in_keeps_its_price() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        serde_json::json!({"providers": {"openai-api": {
            "api": "openai-completions",
            "baseUrl": "https://api.openai.test/v1",
            "apiKey": "sk-test-not-real",
            "models": []
        }}})
        .to_string(),
    )
    .unwrap();
    DiscoverySourceCache::new(&discovery_cache_dir(tmp.path()), "openai-api")
        .store_models_response(r#"{"data":[{"id":"gpt-6-sol","name":"Listed"}]}"#)
        .unwrap();
    let resolved = crate::composition::catalogue::resolve_catalogue_for(tmp.path());
    let reference = crate::domain::catalogue::value_objects::identity::ModelRef::parse_qualified(
        "openai-api/gpt-6-sol",
    )
    .unwrap();
    let entry = resolved.snapshot.find(&reference).expect("published");
    assert_eq!(
        entry.model.display_name.as_deref(),
        Some("Listed"),
        "the discovered entry is the one published"
    );
    let limits = published_model_limits_for(tmp.path(), "openai-api/gpt-6-sol");
    assert_eq!(limits.price, micro(2.0, 10.0, 0.2, 2.5));
}

/// A model nothing prices stays unpriced.
#[test]
fn a_model_no_table_knows_is_unpriced() {
    let tmp = tempfile::tempdir().unwrap();
    let limits = published_model_limits_for(tmp.path(), "anthropic-api/claude-unknown-9");
    assert_eq!(limits.price, None);
}

/// An entry on a transport the harness cannot run is listed with the
/// published price of its id too, never a zero one.
#[test]
fn an_unsupported_transport_entry_lists_its_published_price() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"x":{"baseUrl":"https://example.test/v1","apiKey":"sk-test-not-real","api":"cohere-chat","models":[{"id":"grok-4.5"}]}}}"#,
    )
    .unwrap();
    let resolved = crate::composition::catalogue::resolve_catalogue_for(tmp.path());
    let reference =
        crate::domain::catalogue::value_objects::identity::ModelRef::parse_qualified("x/grok-4.5")
            .unwrap();
    let entry = resolved.snapshot.find(&reference).expect("listed");
    let cost = &entry.model.capabilities.cost;
    assert_eq!(
        (cost.input, cost.output, cost.cache_read, cost.cache_write),
        (2.0, 6.0, 0.3, 2.0)
    );
}

/// #2635 review M1: a `models.json` model declared free (every rate zero)
/// is priced at zero, never left unpriced.
#[test]
fn a_model_declared_free_is_priced_at_zero() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"local":{"api":"openai-completions","baseUrl":"http://127.0.0.1:8080/v1","models":[
            {"id":"free-model","cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0}},
            {"id":"unstated-model"},
            {"id":"input-only-model","cost":{"input":3}}
        ]}}}"#,
    )
    .unwrap();
    let free = published_model_limits_for(tmp.path(), "local/free-model");
    assert_eq!(free.price, micro(0.0, 0.0, 0.0, 0.0));
    let unstated = published_model_limits_for(tmp.path(), "local/unstated-model");
    assert_eq!(unstated.price, None);
    // #2630 review: an input rate alone would bill output at $0: unpriced.
    let partial = published_model_limits_for(tmp.path(), "local/input-only-model");
    assert_eq!(partial.price, None);
}
