//! Unit tests for the discovery source cache (epic #1193, slice 4).

use super::*;
use crate::domain::catalogue::ModelRef;
use tempfile::TempDir;

#[test]
fn store_persists_models_and_load_round_trips_as_discovered_source() {
    let dir = TempDir::new().unwrap();
    let cache = DiscoverySourceCache::new(dir.path(), "openrouter");

    let stored = cache
        .store_models_response(r#"{"data":[{"id":"alpha"},{"id":"beta"}]}"#)
        .expect("store must succeed");
    assert_eq!(stored, 2, "both models must be persisted");
    assert!(
        cache.cache_path().is_file(),
        "the source cache file must exist"
    );

    assert_eq!(cache.layer(), SourceLayer::Discovered);
    let loaded = cache.load().expect("load must succeed");
    assert_eq!(
        loaded.entries.len(),
        2,
        "load must round-trip the cached models"
    );
    let alpha = ModelRef::parse_qualified("openrouter/alpha").unwrap();
    assert!(
        loaded.entries.iter().any(|e| e.model.reference == alpha),
        "cached entries must be qualified under the provider"
    );
}

#[test]
fn persisted_cache_never_contains_secret_material() {
    let dir = TempDir::new().unwrap();
    let cache = DiscoverySourceCache::new(dir.path(), "openrouter");

    // A hostile or buggy server may echo credential material in the response
    // body; only mapped model entries may reach disk.
    let body = r#"{"data":[{"id":"alpha","api_key":"sk-echoed-secret-123"}],"debug_token":"sk-echoed-secret-123"}"#;
    cache
        .store_models_response(body)
        .expect("store must succeed");

    let persisted =
        std::fs::read_to_string(cache.cache_path()).expect("the source cache file must exist");
    assert!(
        !persisted.contains("sk-echoed-secret-123"),
        "persisted source cache leaked secret material: {persisted}"
    );
    assert!(
        persisted.contains("alpha"),
        "the model itself must be cached"
    );
}
