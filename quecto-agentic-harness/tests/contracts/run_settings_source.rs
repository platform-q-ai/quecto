//! Contract for the configuration capability's `RunSettingsSource`
//! (#1843) over the `Config` schema: a document yields the settings a run
//! would use, defaults applied and `~` expanded, keys by presence only;
//! a document the schema refuses yields its words.
use quecto::application::configuration::ports::RunSettingsSource;
use quecto::infrastructure::config::mapping::ConfigRunSettings;
use serde_json::json;

fn source() -> (tempfile::TempDir, ConfigRunSettings) {
    let base = tempfile::TempDir::new().unwrap();
    let source = ConfigRunSettings::for_base_dir(base.path());
    (base, source)
}

#[test]
fn an_empty_document_yields_the_defaults() {
    let (_base, source) = source();
    let settings = source.settings(json!({})).unwrap();
    assert!(!settings.model.is_empty(), "a default model applies");
    assert_eq!(settings.effort, None);
    assert!(!settings.openai_key && !settings.anthropic_key);
    assert!(settings.endpoint_keys.is_empty());
    assert!(
        !settings.workspace.starts_with('~'),
        "{}",
        settings.workspace
    );
}

#[test]
fn the_documents_values_and_key_presence_come_through() {
    let (_base, source) = source();
    let settings = source
        .settings(json!({
            "agents": {"defaults": {"model": "acme/m", "effort": "high", "workspace": "/w"}},
            "providers": {
                "openai": {"api_key": "fake-openai"},
                "anthropic": {"api_key": ""},
                "openai_compatible": {"endpoints": [
                    {"prefix": "keyed", "api_key": "fake-ep", "api_base": "https://x.test/v1"},
                    {"prefix": "keyless", "api_key": "", "api_base": "https://y.test/v1"}
                ]}
            }
        }))
        .unwrap();
    assert_eq!(settings.model, "acme/m");
    assert_eq!(settings.effort.as_deref(), Some("high"));
    assert_eq!(settings.workspace, "/w");
    assert!(settings.openai_key);
    assert!(!settings.anthropic_key, "an empty key is no key");
    assert_eq!(settings.endpoint_keys, ["keyed"]);
    assert!(
        !format!("{settings:?}").contains("fake-"),
        "no key material"
    );
}

#[test]
fn a_document_the_schema_refuses_yields_its_words() {
    let (_base, source) = source();
    let refused = source
        .settings(json!({"agents": {"defaults": {"model": 42}}}))
        .expect_err("a number is no model");
    assert!(refused.contains("invalid type"), "{refused}");
}
