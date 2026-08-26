use super::*;
use tempfile::TempDir;

fn configured_runtime(_temp: &TempDir) -> (Config, reqwest::Client) {
    let mut config = Config::default();
    config.providers.openai.api_key = "test-key".into();
    config.providers.openai.api_base = "https://api.openai.com/v1".into();
    (config, reqwest::Client::new())
}

#[test]
fn runtime_snapshot_keeps_requested_generation_and_descriptors() {
    let temp = TempDir::new().unwrap();
    let (config, client) = configured_runtime(&temp);
    let snapshot = build_runtime_snapshot(&config, temp.path(), &client, 17).unwrap();
    assert_eq!(snapshot.generation(), 17);
    assert!(!snapshot.catalogue.models().is_empty());
    assert_eq!(
        snapshot.catalogue.models().len(),
        snapshot.provider.model_descriptors().unwrap().len()
    );
}

#[test]
fn runtime_snapshot_reports_missing_provider_configuration() {
    let temp = TempDir::new().unwrap();
    let error = build_runtime_snapshot(&Config::default(), temp.path(), &reqwest::Client::new(), 0)
        .unwrap_err();
    assert!(error.contains("no LLM providers configured"));
}

#[test]
fn every_constructed_provider_is_published_as_a_routable_prefix() {
    use crate::infrastructure::config::OpenAiCompatibleEndpoint;

    let temp = TempDir::new().unwrap();
    let (mut config, client) = configured_runtime(&temp);
    config.providers.openai_compatible.endpoints = vec![
        OpenAiCompatibleEndpoint {
            prefix: " spark ".to_string(),
            api_key: "sk-endpoint".to_string(),
            api_base: "http://127.0.0.1:9/v1".to_string(),
            allow_remote_http: true,
        },
        OpenAiCompatibleEndpoint {
            prefix: "keyless".to_string(),
            api_key: String::new(),
            api_base: "http://127.0.0.1:9/v1".to_string(),
            allow_remote_http: true,
        },
    ];

    let snapshot = build_runtime_snapshot(&config, temp.path(), &client, 3).unwrap();

    let open: Vec<_> = snapshot
        .catalogue
        .open_providers()
        .iter()
        .map(|provider| provider.as_str().to_string())
        .collect();
    // Every constructed provider routes ids the catalogue may not enumerate,
    // and a keyless endpoint constructs nothing.
    assert!(open.contains(&"spark".to_string()));
    assert!(open.contains(&"openai-api".to_string()));
    assert!(
        !open.contains(&"keyless".to_string()),
        "a keyless endpoint constructs no provider: {open:?}"
    );
}
