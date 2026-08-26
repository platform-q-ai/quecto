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
    let source = ModelsFileCatalogueSource::preloaded(Ok(vec![record]));

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

    let credentials = RegistryCredentialStatus::from_records([&configured, &unconfigured]);
    let configured_entry = entry_from_record(&configured).unwrap();
    let unconfigured_entry = entry_from_record(&unconfigured).unwrap();

    assert!(credentials.credential_available(&configured_entry));
    assert!(!credentials.credential_available(&unconfigured_entry));
}

#[test]
fn snapshot_store_for_reuses_store_per_base_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let first = snapshot_store_for(tmp.path());
    first.publish(crate::domain::catalogue::CatalogueSnapshot::empty(41));

    let second = snapshot_store_for(tmp.path());

    assert_eq!(second.current().generation(), 41);
}
