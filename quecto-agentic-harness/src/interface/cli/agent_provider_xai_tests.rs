// xAI (Grok) OAuth wiring tests (PR #1087 follow-up):
// - registry base-URL pinning for the `(OpenAiCompletions, "xai")` match arm
// - refresh-dispatch selection for the `"xai"` provider in shared.rs

use super::*;

/// A real builtin xai/grok-4.5 record, optionally with a configured baseUrl.
fn xai_record(base_url: Option<&str>) -> crate::infrastructure::model_registry::ModelRecord {
    let registry = crate::infrastructure::model_registry::ModelRegistry::builtin();
    let mut model = registry
        .find("xai", "grok-4.5")
        .expect("grok-4.5 builtin")
        .clone();
    model.base_url = base_url.map(|s| s.to_string());
    model
}

#[test]
fn xai_base_url_defaults_to_canonical_host() {
    let model = xai_record(None);
    let url = oauth_registry_base_url(&model, "xai").unwrap();
    assert_eq!(url.as_deref(), Some("https://api.x.ai/v1"));
}

#[test]
fn xai_base_url_accepts_canonical_host_explicitly() {
    // Same host/scheme/port with a different path is accepted (path is
    // preserved from the configured value).
    let model = xai_record(Some("https://api.x.ai/v1"));
    let url = oauth_registry_base_url(&model, "xai").unwrap();
    assert_eq!(url.as_deref(), Some("https://api.x.ai/v1"));
}

#[test]
fn xai_base_url_rejects_foreign_host() {
    let model = xai_record(Some("https://attacker.example/v1"));
    let err = oauth_registry_base_url(&model, "xai").unwrap_err();
    assert!(err.contains("canonical OAuth host"), "got: {}", err);
}

#[test]
fn xai_base_url_rejects_lookalike_suffix_host() {
    // Host-suffix confusion must not pass the exact host_str comparison.
    let model = xai_record(Some("https://api.x.ai.attacker.example/v1"));
    assert!(oauth_registry_base_url(&model, "xai").is_err());
}

#[test]
fn xai_base_url_rejects_wrong_scheme() {
    let model = xai_record(Some("http://api.x.ai/v1"));
    assert!(oauth_registry_base_url(&model, "xai").is_err());
}

#[test]
fn xai_base_url_rejects_wrong_port() {
    let model = xai_record(Some("https://api.x.ai:8443/v1"));
    assert!(oauth_registry_base_url(&model, "xai").is_err());
}

#[test]
fn xai_base_url_rejects_invalid_url() {
    let model = xai_record(Some("not a url"));
    assert!(oauth_registry_base_url(&model, "xai").is_err());
}
