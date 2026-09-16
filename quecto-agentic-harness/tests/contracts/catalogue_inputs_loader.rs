//! Contract for the `CatalogueInputsLoader` port (#1845): every load is a
//! fresh read of the base directory's inputs, so an edit to `models.json`
//! is visible on the next load. What one load promises is the
//! `LoadedCatalogueInputs` contract.
use std::sync::Arc;

use quecto::application::catalogue::ports::CatalogueInputsLoader;
use quecto::domain::catalogue::SourceLayer;
use quecto::infrastructure::catalogue_inputs::FileCatalogueInputs;

fn under_test(dir: &std::path::Path) -> Arc<dyn CatalogueInputsLoader> {
    Arc::new(FileCatalogueInputs::new(dir))
}

#[test]
fn each_load_reads_the_current_models_json() {
    let tmp = tempfile::tempdir().unwrap();
    let loader = under_test(tmp.path());
    let before = loader.load();
    let user_before: usize = before
        .sources()
        .iter()
        .filter(|s| s.layer() == SourceLayer::UserDefined)
        .map(|s| s.load().unwrap().entries.len())
        .sum();
    assert_eq!(user_before, 0);

    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"acme":{"api":"openai-completions","baseUrl":"https://acme.test/v1","apiKey":"$ACME_KEY","models":[{"id":"acme-1"}]}}}"#,
    )
    .unwrap();
    let after = loader.load();
    let user_after: usize = after
        .sources()
        .iter()
        .filter(|s| s.layer() == SourceLayer::UserDefined)
        .map(|s| s.load().unwrap().entries.len())
        .sum();
    assert_eq!(user_after, 1, "the edit is visible on the next load");
}
