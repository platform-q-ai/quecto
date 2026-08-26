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
    assert_eq!(
        stored,
        RefreshChange::Updated { models: 2 },
        "both models must be persisted"
    );
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

#[test]
fn malformed_response_fails_and_preserves_the_previous_cache() {
    let dir = TempDir::new().unwrap();
    let cache = DiscoverySourceCache::new(dir.path(), "openrouter");
    cache
        .store_models_response(r#"{"data":[{"id":"alpha"}]}"#)
        .expect("seeding the cache must succeed");
    let before = std::fs::read_to_string(cache.cache_path()).unwrap();

    for body in ["not json", r#"{"models":[]}"#, r#"{"data":[{"id":42}]}"#] {
        cache
            .store_models_response(body)
            .expect_err("a malformed body must be an error");
        let after = std::fs::read_to_string(cache.cache_path()).unwrap();
        assert_eq!(after, before, "a failed store must leave the cache intact");
    }
}

#[test]
fn empty_model_list_persists_an_empty_cache() {
    let dir = TempDir::new().unwrap();
    let cache = DiscoverySourceCache::new(dir.path(), "openrouter");
    let stored = cache
        .store_models_response(r#"{"data":[]}"#)
        .expect("an empty listing is a valid response");
    assert_eq!(stored, RefreshChange::Updated { models: 0 });
    let loaded = cache.load().expect("load must succeed");
    assert!(
        loaded.entries.is_empty(),
        "an empty cache must load as an empty discovered layer"
    );
}

#[test]
fn response_at_the_byte_cap_is_read_in_full() {
    let body = vec![b'x'; 1024];
    let read = read_capped(body.as_slice(), 1024).expect("a body at the cap must be accepted");
    assert_eq!(read.len(), 1024);
}

#[test]
fn response_over_the_byte_cap_is_rejected() {
    let body = vec![b'x'; 1025];
    let error = read_capped(body.as_slice(), 1024).expect_err("a body over the cap must error");
    assert!(error.contains("1024"), "got: {error}");
}

#[test]
fn stored_models_are_deduplicated_by_id_with_last_writer_winning() {
    let dir = TempDir::new().unwrap();
    let cache = DiscoverySourceCache::new(dir.path(), "openrouter");
    cache
        .store_models_response(
            r#"{"data":[{"id":"alpha","name":"First"},{"id":"alpha","name":"Last"}]}"#,
        )
        .expect("store must succeed");
    let loaded = cache.load().unwrap();
    assert_eq!(loaded.entries.len(), 1, "duplicate ids must collapse");
    assert_eq!(
        loaded.entries[0].model.display_name.as_deref(),
        Some("Last")
    );
}

#[test]
fn oversized_model_catalogue_is_rejected() {
    let dir = TempDir::new().unwrap();
    let cache = DiscoverySourceCache::new(dir.path(), "openrouter");
    let data: Vec<String> = (0..=10_000)
        .map(|i| format!(r#"{{"id":"model-{i}"}}"#))
        .collect();
    let body = format!(r#"{{"data":[{}]}}"#, data.join(","));
    let err = cache
        .store_models_response(&body)
        .expect_err("an oversized catalogue must be rejected");
    assert!(err.contains("more than"), "got: {err}");
}

#[test]
fn discovery_endpoint_derivation_applies_url_policy() {
    let endpoint = DiscoveryEndpoint::for_openai_compatible(
        "remote-http",
        "http://example.invalid/inference/v1",
        true,
        None,
    )
    .expect("explicitly allowed remote http must derive");
    assert_eq!(endpoint.url, "http://example.invalid/inference/v1/models");

    let err = DiscoveryEndpoint::for_openai_compatible(
        "plain",
        "http://attacker.example/v1",
        false,
        None,
    )
    .expect_err("remote http must be rejected by default");
    assert!(err.contains("loopback"), "got: {err}");
}

#[test]
fn error_urls_are_redacted() {
    assert_eq!(
        redact_url_for_error("https://user:pass@example.com/v1/models?token=secret#fragment"),
        "https://example.com/v1/models"
    );
}

/// Slice-4 review: the provider key becomes the cache file stem, so a key
/// that could traverse outside the cache dir (or nest a subdirectory the
/// enumerator would never re-list) must be refused before touching disk.
#[test]
fn unsafe_provider_keys_never_reach_the_filesystem() {
    let dir = TempDir::new().unwrap();
    for key in [
        "../../escape",
        "nested/provider",
        "back\\slash",
        ".hidden",
        "",
    ] {
        let cache = DiscoverySourceCache::new(dir.path(), key);
        let err = cache
            .store_models_response(r#"{"data":[{"id":"alpha"}]}"#)
            .expect_err("an unsafe key must be rejected");
        assert!(
            err.contains("cannot name a discovery cache file"),
            "got: {err}"
        );
    }
    assert!(
        std::fs::read_dir(dir.path()).unwrap().next().is_none(),
        "no cache file may be created for an unsafe key"
    );
}

/// Slice-4 review: a refresh whose mapped listing equals the existing cache
/// reports Unchanged (with the cached count) and skips the rewrite entirely.
#[test]
fn identical_listing_is_unchanged_and_skips_the_cache_rewrite() {
    let dir = TempDir::new().unwrap();
    let cache = DiscoverySourceCache::new(dir.path(), "openrouter");
    let body = r#"{"data":[{"id":"alpha"},{"id":"beta"}]}"#;
    assert_eq!(
        cache.store_models_response(body).unwrap(),
        RefreshChange::Updated { models: 2 }
    );
    let mtime_before = std::fs::metadata(cache.cache_path())
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(
        cache.store_models_response(body).unwrap(),
        RefreshChange::Unchanged { models: 2 },
        "an identical listing must be reported unchanged with the cached count"
    );
    let mtime_after = std::fs::metadata(cache.cache_path())
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(
        mtime_before, mtime_after,
        "an unchanged listing must not rewrite the cache file"
    );
}
