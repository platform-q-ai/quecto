//! Tests for the catalogue source/credential adapters over the legacy
//! registry data (issue #1572).

use super::*;

#[test]
fn builtin_source_maps_the_builtin_table_into_domain_entries() {
    let entries = BuiltinCatalogueSource.load().unwrap().entries;
    let builtin = ModelRegistry::builtin();
    assert_eq!(entries.len(), builtin.models().len());
    let first = &builtin.models()[0];
    assert_eq!(
        entries[0].reference().qualified_id(),
        format!("{}/{}", first.provider, first.id)
    );
    assert_eq!(BuiltinCatalogueSource.layer(), SourceLayer::BuiltIn);
}

#[test]
fn models_file_source_loads_only_the_file_records() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"custom":{"api":"openai-completions","apiKey":"sk-x",
            "models":[{"id":"m1","name":"M One"}]}}}"#,
    )
    .unwrap();
    let source = ModelsFileCatalogueSource::new(tmp.path());
    let entries = source.load().unwrap().entries;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].reference().qualified_id(), "custom/m1");
    assert_eq!(source.layer(), SourceLayer::UserDefined);
}

#[test]
fn models_file_source_reports_a_parse_failure_as_a_load_error() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("models.json"), "not json").unwrap();
    let error = ModelsFileCatalogueSource::new(tmp.path())
        .load()
        .unwrap_err();
    assert!(
        error.contains("failed to parse"),
        "unexpected error: {error}"
    );
}

#[test]
fn missing_models_file_is_an_empty_layer() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(
        ModelsFileCatalogueSource::new(tmp.path())
            .load()
            .unwrap()
            .entries
            .is_empty()
    );
}

#[test]
fn preloaded_models_file_source_uses_records_without_file_io() {
    let mut record = ModelRegistry::builtin().models()[0].clone();
    record.provider = "preloaded-provider".to_string();
    record.id = "preloaded-model".to_string();
    let source = ModelsFileCatalogueSource::preloaded(Ok(entries_from_records(&[record])));

    let loaded = source.load().unwrap();

    assert_eq!(loaded.entries.len(), 1);
    assert_eq!(
        loaded.entries[0].reference().qualified_id(),
        "preloaded-provider/preloaded-model"
    );
    assert!(loaded.skipped.is_empty());
}

#[test]
fn preloaded_models_file_source_surfaces_parse_error() {
    let source = ModelsFileCatalogueSource::preloaded(Err("bad json".to_string()));

    assert_eq!(source.load().unwrap_err(), "bad json");
}

#[test]
fn registry_credentials_are_keyed_by_qualified_model() {
    let mut configured = ModelRegistry::builtin().models()[0].clone();
    configured.provider = "same-provider".to_string();
    configured.id = "configured".to_string();
    configured.api_key = Some("sk-test".to_string());
    let mut unconfigured = configured.clone();
    unconfigured.id = "missing".to_string();
    unconfigured.api_key = None;
    unconfigured.base_url = None;

    let credentials = RegistryCredentialStatus::new([&configured, &unconfigured], no_slots());
    let configured_entry = entry_from_record(&configured).unwrap();
    let unconfigured_entry = entry_from_record(&unconfigured).unwrap();

    assert!(credentials.credential_available(&configured_entry));
    assert!(!credentials.credential_available(&unconfigured_entry));
}

fn no_slots() -> ProviderSlots {
    ProviderSlots::none()
}

fn builtin_entry(provider: &str, id: &str) -> CatalogueEntry {
    entry_from_record(
        ModelRegistry::builtin()
            .find(provider, id)
            .expect("built in"),
    )
    .unwrap()
}

#[test]
fn a_builtin_is_credentialed_by_its_slot_not_by_its_record() {
    let builtin = ModelRegistry::builtin();
    let oauth = builtin_entry("anthropic-oauth", "claude-opus-5-5");
    let api = builtin_entry("anthropic-api", "claude-opus-5-5");

    let without = RegistryCredentialStatus::new(builtin.models(), no_slots());
    assert!(!without.credential_available(&oauth));
    assert!(!without.credential_available(&api));

    let signed_in = RegistryCredentialStatus::new(
        builtin.models(),
        ProviderSlots::routed(["anthropic-oauth".to_string()]),
    );
    assert!(signed_in.credential_available(&oauth));
    assert!(
        !signed_in.credential_available(&api),
        "an OAuth slot never credits the API slot"
    );
}

#[test]
fn an_oauth_record_is_credentialed_by_its_sign_in_not_its_own_key_or_url() {
    let mut record = ModelRegistry::builtin()
        .find("anthropic-oauth", "claude-opus-5-5")
        .unwrap()
        .clone();
    record.provider = "my-claude".to_string();
    record.api_key = Some("sk-ignored".to_string());
    record.base_url = Some("https://api.anthropic.com".to_string());
    let entry = entry_from_record(&record).unwrap();

    assert!(!RegistryCredentialStatus::new([&record], no_slots()).credential_available(&entry));
    assert!(
        RegistryCredentialStatus::new([&record], ProviderSlots::routed(["my-claude".to_string()]))
            .credential_available(&entry)
    );
}

#[test]
fn a_routed_api_key_provider_does_not_credit_a_keyless_sibling() {
    let mut keyed = ModelRegistry::builtin().models()[0].clone();
    keyed.provider = "custom".to_string();
    keyed.id = "keyed".to_string();
    keyed.auth = crate::infrastructure::model_registry::AuthMode::ApiKey;
    keyed.oauth_provider = None;
    keyed.api_key = Some("sk-test".to_string());
    let mut keyless = keyed.clone();
    keyless.id = "keyless".to_string();
    keyless.api_key = None;
    keyless.base_url = None;

    // The runtime routes `custom` (built from the keyed record), yet the
    // keyless sibling stays uncredentialed.
    let credentials = RegistryCredentialStatus::new(
        [&keyed, &keyless],
        ProviderSlots::routed(["custom".to_string()]),
    );
    assert!(credentials.credential_available(&entry_from_record(&keyed).unwrap()));
    assert!(!credentials.credential_available(&entry_from_record(&keyless).unwrap()));
}

#[test]
fn snapshot_store_for_reuses_store_per_base_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let first = snapshot_store_for(tmp.path());
    first.publish(crate::domain::catalogue::CatalogueSnapshot::empty(41));

    let second = snapshot_store_for(tmp.path());

    assert_eq!(second.current().generation(), 41);
}
