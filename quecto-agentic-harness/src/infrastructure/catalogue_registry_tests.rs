//! Tests for the catalogue source/credential adapters over the legacy
//! registry data (issue #1572).

use super::*;

#[test]
fn builtin_source_maps_the_builtin_table_into_domain_entries() {
    let entries = BuiltinCatalogueSource.load().unwrap();
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
    let entries = source.load().unwrap();
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
            .is_empty()
    );
}
