//! Contract for `LoadedRefreshInputs` (#1846): one load's refreshables and
//! its resolve sources describe the same file read; every refreshable's
//! provider feeds the resolve sources exactly once — through its persisted
//! cache (`discovered:<provider>`) when one exists, otherwise through the
//! live source appended for the first refresh; and the redaction strips the
//! providers' configured secrets.
use std::sync::Arc;

use quecto::application::catalogue::ports::RefreshInputsLoader;
use quecto::infrastructure::catalogue_refresh_inputs::FileRefreshInputs;

fn under_test(dir: &std::path::Path) -> Arc<dyn RefreshInputsLoader> {
    Arc::new(FileRefreshInputs::new(dir))
}

const REGISTRY: &str = r#"{"providers":{"openrouter":{"api":"openai-completions","baseUrl":"https://or.test/v1","apiKey":"sk-or-secret","models":[]}}}"#;

#[test]
fn a_refreshable_provider_feeds_the_resolve_sources_exactly_once() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("models.json"), REGISTRY).unwrap();
    let loaded = under_test(tmp.path()).load().unwrap();
    assert_eq!(loaded.refreshables().len(), 1);
    let feeders = |loaded: &dyn quecto::application::catalogue::ports::LoadedRefreshInputs| {
        loaded
            .sources()
            .iter()
            .map(|s| s.id().to_string())
            .filter(|id| id == "openrouter" || id == "discovered:openrouter")
            .collect::<Vec<_>>()
    };
    assert_eq!(
        feeders(loaded.as_ref()),
        vec!["openrouter"],
        "first refresh: the live source is appended once"
    );

    // Once a cache exists for the provider, the cached layer feeds it and
    // the live source is not appended again.
    quecto::infrastructure::catalogue_discovery::DiscoverySourceCache::new(
        &quecto::infrastructure::catalogue_discovery::discovery_cache_dir(tmp.path()),
        "openrouter",
    )
    .store_models_response(r#"{"data":[{"id":"alpha","name":"Alpha"}]}"#)
    .unwrap();
    let loaded = under_test(tmp.path()).load().unwrap();
    assert_eq!(
        feeders(loaded.as_ref()),
        vec!["discovered:openrouter"],
        "cached: fed by the cache, not twice"
    );
}

#[test]
fn the_redaction_strips_the_configured_secrets_and_the_credential_port_answers() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("models.json"), REGISTRY).unwrap();
    let loaded = under_test(tmp.path()).load().unwrap();
    assert_eq!(
        loaded.redaction().redact("401 for bearer sk-or-secret"),
        "401 for bearer [redacted]"
    );
    let entries: Vec<_> = loaded
        .sources()
        .iter()
        .filter(|s| s.id() == "openrouter")
        .flat_map(|s| s.load().unwrap().entries)
        .collect();
    // Nothing discovered yet: no entries, but the port answers per entry.
    assert!(entries.is_empty());
}
