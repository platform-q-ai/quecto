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
    assert_eq!(parsed.records[0].stream_limits.idle_seconds, Some(900));
    let defaults: Vec<_> = parsed
        .providers
        .iter()
        .map(|(key, d)| (key.as_str(), d.stream_limits.idle_seconds))
        .collect();
    assert_eq!(defaults, [("fine", Some(900))]);
}

/// Whether a provider was built with an idle bound of `idle` and a
/// progress bound of `progress` seconds, as its debug form shows them.
fn shows_bounds(provider: &dyn LlmProvider, idle: u64, progress: u64) -> bool {
    let secs = std::time::Duration::from_secs;
    let bounds = StreamIdle::new(secs(idle)).with_progress(secs(progress));
    format!("{provider:?}").contains(&format!("{bounds:?}"))
}

/// Whether a provider was built with an idle bound of `seconds` and the
/// default progress bound.
fn shows(provider: &dyn LlmProvider, seconds: u64) -> bool {
    shows_bounds(provider, seconds, 300)
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
        "streamProgressSeconds": 1200, "models": [{"id": "m"}],
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
    assert!(shows_bounds(&*built, 900, 1200), "{built:?}");
    record.stream_limits = Default::default();
    let built = build_registry_provider(&record, tmp.path(), &store, &refresh, &client)
        .unwrap()
        .unwrap();
    assert!(shows(&*built, 300), "{built:?}");

    let limits = crate::infrastructure::providers::stream_idle::StreamLimits {
        idle_seconds: Some(1200),
        progress_seconds: None,
    };
    let idle = StreamIdle::configured(limits).unwrap();
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
fn the_allowed_ranges_are_half_a_minute_to_half_an_hour_and_a_minute_to_an_hour() {
    use crate::infrastructure::providers::stream_idle::StreamLimits;
    let idle = |seconds| StreamLimits {
        idle_seconds: Some(seconds),
        progress_seconds: None,
    };
    let progress = |seconds| StreamLimits {
        idle_seconds: None,
        progress_seconds: Some(seconds),
    };
    for (seconds, allowed) in [(29, false), (30, true), (1800, true), (1801, false)] {
        assert_eq!(
            StreamIdle::configured(idle(seconds)).is_ok(),
            allowed,
            "{seconds}"
        );
    }
    for (seconds, allowed) in [(59, false), (60, true), (3600, true), (3601, false)] {
        let configured = StreamIdle::configured(progress(seconds));
        assert_eq!(configured.is_ok(), allowed, "{seconds}");
    }
    let unset = StreamIdle::configured(StreamLimits::default());
    assert_eq!(unset, Ok(StreamIdle::default()));
    let err = StreamIdle::configured_for("providers.openai", idle(5)).unwrap_err();
    assert_eq!(
        err,
        "providers.openai: stream_idle_seconds must be within 30–1800 seconds, got 5"
    );
    let err = StreamIdle::configured_for("providers.openai", progress(5)).unwrap_err();
    assert_eq!(
        err,
        "providers.openai: stream_progress_seconds must be within 60–3600 seconds, got 5"
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

/// #2433 review round 3: the limits live in their own module but stay flat
/// in their entry, and an unset one is not written.
#[test]
fn the_limits_are_read_and_written_flat_in_their_entry() {
    use crate::infrastructure::config::{OpenAiCompatibleEndpoint, ProviderEntry};
    let entry: ProviderEntry = serde_json::from_value(serde_json::json!({
        "api_key": "k", "stream_idle_seconds": 600, "stream_progress_seconds": 900,
    }))
    .unwrap();
    assert_eq!(entry.stream_limits.stream_idle_seconds, Some(600));
    assert_eq!(entry.stream_limits.stream_progress_seconds, Some(900));
    let written = serde_json::to_value(&entry).unwrap();
    assert_eq!(written["stream_idle_seconds"], 600);
    assert_eq!(written["stream_progress_seconds"], 900);
    assert!(written.get("stream_limits").is_none(), "{written}");
    let unset = serde_json::to_value(OpenAiCompatibleEndpoint::default()).unwrap();
    assert!(unset.get("stream_idle_seconds").is_none(), "{unset}");
    assert!(unset.get("stream_progress_seconds").is_none(), "{unset}");
    let bounds = entry.stream_limits.bounds("providers.openai").unwrap();
    assert_eq!(bounds.limit(), std::time::Duration::from_secs(600));
    assert_eq!(bounds.progress(), std::time::Duration::from_secs(900));
}
