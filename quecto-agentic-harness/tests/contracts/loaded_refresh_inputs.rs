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
fn the_redaction_strips_the_configured_secrets() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("models.json"), REGISTRY).unwrap();
    let loaded = under_test(tmp.path()).load().unwrap();
    assert_eq!(
        loaded.redaction().redact("401 for bearer sk-or-secret"),
        "401 for bearer [redacted]"
    );
}

#[test]
fn the_credential_port_answers_per_entry_of_the_loaded_sources() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{
            "keyed":{"api":"openai-completions","baseUrl":"https://k.test/v1","apiKey":"sk-k","models":[{"id":"m"}]},
            "bare":{"api":"openai-completions","models":[{"id":"m"}]}
        }}"#,
    )
    .unwrap();
    let loaded = under_test(tmp.path()).load().unwrap();
    let entries: Vec<_> = loaded
        .sources()
        .iter()
        .flat_map(|s| s.load().unwrap().entries)
        .collect();
    let find = |provider: &str| {
        entries
            .iter()
            .find(|e| e.reference().provider().as_str() == provider)
            .unwrap_or_else(|| panic!("{provider}/m loads"))
    };
    assert!(loaded.credentials().credential_available(find("keyed")));
    assert!(!loaded.credentials().credential_available(find("bare")));
}
