//! #2433 review: the stream idle limit is configurable per provider, as
//! `stream_idle_seconds` in config.json's provider entries and
//! `streamIdleSeconds` in a models.json provider block, within 30–1800 s.
use super::*;

fn inputs(base_dir: &std::path::Path) -> AgentRuntimeInputs {
    let client = reqwest::Client::new();
    AgentRuntimeInputs {
        base_dir: base_dir.to_path_buf(),
        http_client: client.clone(),
        refresh_fn: crate::infrastructure::providers::refresh_wiring::make_oauth_refresh_fn(),
        openai_oauth_factory:
            crate::infrastructure::providers::refresh_wiring::make_provider_factory(
                "openai", None, client,
            ),
        model_registry: crate::infrastructure::model_registry::ModelRegistry::load_from_path(
            &base_dir.join("models.json"),
        )
        .map_err(|e| e.to_string()),
    }
}

/// A config whose `providers` section is `providers`.
fn config(providers: serde_json::Value) -> Config {
    serde_json::from_value(serde_json::json!({ "providers": providers })).unwrap()
}

fn compose(config: &Config, base_dir: &std::path::Path) -> Result<(), String> {
    compose_agent_provider(config, &inputs(base_dir)).map(|_| ())
}

/// A provider entry's limit outside 30–1800 s fails the composition,
/// naming the setting; the ends of the range are accepted.
#[test]
fn a_configured_limit_out_of_range_fails_the_composition() {
    let tmp = tempfile::TempDir::new().unwrap();
    for (slot, base) in [
        ("openai", "http://127.0.0.1:9/v1"),
        ("anthropic", "http://127.0.0.1:9"),
    ] {
        let entry = |seconds: u64| {
            config(serde_json::json!({
                slot: {"api_key": "sk-test", "api_base": base, "stream_idle_seconds": seconds}
            }))
        };
        for seconds in [0, 29, 1801] {
            let err = compose(&entry(seconds), tmp.path()).unwrap_err();
            assert!(
                err.contains("stream_idle_seconds"),
                "{slot} {seconds}: {err}"
            );
            assert!(err.contains("30") && err.contains("1800"), "{slot}: {err}");
        }
        for seconds in [30, 1800] {
            compose(&entry(seconds), tmp.path()).unwrap();
        }
    }
    let endpoint = |seconds: u64| {
        config(serde_json::json!({"openai_compatible": {"endpoints": [{
            "prefix": "gw", "api_key": "k", "api_base": "http://127.0.0.1:9/v1",
            "stream_idle_seconds": seconds,
        }]}}))
    };
    let err = compose(&endpoint(1801), tmp.path()).unwrap_err();
    assert!(err.contains("stream_idle_seconds"), "{err}");
    compose(&endpoint(600), tmp.path()).unwrap();
}

/// A models.json provider block whose limit is out of range is skipped with
/// a diagnostic naming the setting, as an unknown auth mode is; its
/// neighbours load, a limit in range included.
#[test]
fn a_models_json_block_out_of_range_is_skipped() {
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("models.json");
    let block = |seconds: u64| {
        serde_json::json!({
            "baseUrl": "http://127.0.0.1:9/v1",
            "apiKey": "k",
            "streamIdleSeconds": seconds,
            "models": [{"id": "m"}],
        })
    };
    let file = serde_json::json!({"providers": {"slow": block(10), "fine": block(900)}});
    std::fs::write(&path, file.to_string()).unwrap();
    let parsed =
        crate::infrastructure::model_registry::ModelRegistry::load_registry_config(&path).unwrap();
    let skipped: Vec<_> = parsed.skipped.iter().map(|s| s.provider.as_str()).collect();
    assert_eq!(skipped, ["slow"]);
    assert!(
        parsed.skipped[0].error.contains("streamIdleSeconds"),
        "{}",
        parsed.skipped[0].error
    );
    let loaded: Vec<_> = parsed.records.iter().map(|r| r.provider.as_str()).collect();
    assert_eq!(loaded, ["fine"]);
    assert_eq!(parsed.records[0].stream_idle_seconds, Some(900));
    let defaults: Vec<_> = parsed
        .providers
        .iter()
        .map(|(key, d)| (key.as_str(), d.stream_idle_seconds))
        .collect();
    assert_eq!(defaults, [("fine", Some(900))]);
}

/// The bounds a provider was built with, as its debug form shows them.
fn shows(provider: &dyn LlmProvider, seconds: u64) -> bool {
    let idle = StreamIdle::new(std::time::Duration::from_secs(seconds));
    format!("{provider:?}").contains(&format!("{idle:?}"))
}

/// A configured limit binds the providers built for it: a models.json
/// block's, a config entry's, and an OpenAI OAuth provider rebuilt after a
/// token refresh; an unset one keeps the default.
#[test]
fn a_configured_limit_binds_its_providers() {
    use crate::infrastructure::model_registry::{ModelRecord, ProviderApi};
    let tmp = tempfile::TempDir::new().unwrap();
    let client = reqwest::Client::new();
    let file = serde_json::json!({"providers": {"local": {
        "baseUrl": "http://127.0.0.1:9/v1", "apiKey": "k", "streamIdleSeconds": 900,
        "models": [{"id": "m"}],
    }}});
    let path = tmp.path().join("models.json");
    std::fs::write(&path, file.to_string()).unwrap();
    let records =
        crate::infrastructure::model_registry::ModelRegistry::load_file_records(&path).unwrap();
    let mut record: ModelRecord = records.into_iter().next().unwrap();
    assert_eq!(record.api, ProviderApi::OpenAiCompletions);
    let store = Arc::new(CredentialStore::new(tmp.path()));
    let refresh = crate::infrastructure::providers::refresh_wiring::make_oauth_refresh_fn();
    let built = build_registry_provider(&record, tmp.path(), &store, &refresh, &client)
        .unwrap()
        .unwrap();
    assert!(shows(&*built, 900), "{built:?}");
    record.stream_idle_seconds = None;
    let built = build_registry_provider(&record, tmp.path(), &store, &refresh, &client)
        .unwrap()
        .unwrap();
    assert!(shows(&*built, 300), "{built:?}");

    let idle = StreamIdle::configured(Some(1200)).unwrap();
    let base = Some("http://127.0.0.1:9".to_owned());
    for name in ["openai", "anthropic"] {
        let binding = bound(None, name, idle).unwrap();
        let built = build_single_provider_with_admission(name, "sk", &base, &client, true, binding)
            .unwrap();
        assert!(shows(&*built, 1200), "{name}: {built:?}");
    }
    let rebuild = crate::infrastructure::providers::refresh_wiring::make_bounded_provider_factory(
        "openai", None, idle, client,
    );
    let rebuilt = rebuild("plain-non-jwt-token");
    assert!(shows(&*rebuilt, 1200), "{rebuilt:?}");
}

#[test]
fn the_allowed_range_is_thirty_seconds_to_half_an_hour() {
    for (seconds, allowed) in [(29, false), (30, true), (1800, true), (1801, false)] {
        assert_eq!(
            StreamIdle::configured(Some(seconds)).is_some(),
            allowed,
            "{seconds}"
        );
    }
    assert_eq!(StreamIdle::configured(None), Some(StreamIdle::default()));
    let err = StreamIdle::configured_for("providers.openai", Some(5)).unwrap_err();
    assert_eq!(
        err,
        "providers.openai: stream_idle_seconds must be within 30–1800 seconds, got 5"
    );
}

/// #2433: the output progress limit is configured beside the idle limit,
/// `stream_progress_seconds` / `streamProgressSeconds`, within 60–3600 s.
#[test]
fn a_progress_limit_out_of_range_fails_or_skips() {
    let tmp = tempfile::TempDir::new().unwrap();
    let entry = |seconds: u64| {
        config(serde_json::json!({"openai": {
            "api_key": "sk-test", "api_base": "http://127.0.0.1:9/v1",
            "stream_progress_seconds": seconds,
        }}))
    };
    for seconds in [0, 59, 3601] {
        let err = compose(&entry(seconds), tmp.path()).unwrap_err();
        assert!(err.contains("stream_progress_seconds"), "{seconds}: {err}");
        assert!(err.contains("60") && err.contains("3600"), "{err}");
    }
    for seconds in [60, 3600] {
        compose(&entry(seconds), tmp.path()).unwrap();
    }
    let path = tmp.path().join("models.json");
    let block = |seconds: u64| {
        serde_json::json!({
            "baseUrl": "http://127.0.0.1:9/v1", "apiKey": "k",
            "streamProgressSeconds": seconds, "models": [{"id": "m"}],
        })
    };
    let file = serde_json::json!({"providers": {"slow": block(30), "fine": block(900)}});
    std::fs::write(&path, file.to_string()).unwrap();
    let parsed =
        crate::infrastructure::model_registry::ModelRegistry::load_registry_config(&path).unwrap();
    let skipped: Vec<_> = parsed.skipped.iter().map(|s| s.provider.as_str()).collect();
    assert_eq!(skipped, ["slow"]);
    assert!(parsed.skipped[0].error.contains("streamProgressSeconds"));
}
