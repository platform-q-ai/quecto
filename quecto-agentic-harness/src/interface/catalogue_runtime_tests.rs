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
