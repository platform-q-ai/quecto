//! Contract for the application `CatalogueSource` port (issue #1572): every
//! implementation is a named, layered input whose `load` either yields domain
//! entries or a descriptive error — and parsing stays behind the port.

use quecto::application::ports::CatalogueSource;
use quecto::domain::catalogue::SourceLayer;
use quecto::infrastructure::catalogue_registry::{
    BuiltinCatalogueSource, ModelsFileCatalogueSource,
};

fn assert_source_contract(source: &dyn CatalogueSource, expected_layer: SourceLayer) {
    assert!(!source.id().is_empty(), "sources must be identifiable");
    assert_eq!(source.layer(), expected_layer);
    match source.load() {
        Ok(entries) => {
            for entry in entries {
                // Every loaded entry names its own provider — the domain
                // validation invariant sources are expected to satisfy.
                assert_eq!(entry.reference().provider(), &entry.provider.id);
            }
        }
        Err(error) => assert!(!error.is_empty(), "load errors must be descriptive"),
    }
}

#[test]
fn builtin_source_satisfies_the_contract() {
    let source = BuiltinCatalogueSource;
    assert_source_contract(&source, SourceLayer::BuiltIn);
    assert!(!source.load().unwrap().is_empty());
}

#[test]
fn models_file_source_satisfies_the_contract_when_loadable() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"custom":{"api":"openai-completions","models":[{"id":"m1"}]}}}"#,
    )
    .unwrap();
    let source = ModelsFileCatalogueSource::new(tmp.path());
    assert_source_contract(&source, SourceLayer::UserDefined);
    assert_eq!(source.load().unwrap().len(), 1);
}

#[test]
fn models_file_source_satisfies_the_contract_when_malformed() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("models.json"), "not json").unwrap();
    let source = ModelsFileCatalogueSource::new(tmp.path());
    assert_source_contract(&source, SourceLayer::UserDefined);
    assert!(source.load().unwrap_err().contains("failed to parse"));
}
