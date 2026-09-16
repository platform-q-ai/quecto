use super::*;
use crate::application::catalogue::dto::ModelListingOutcome;

#[test]
fn composed_handles_list_the_real_catalogue_of_a_base_directory() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"acme":{"api":"openai-completions","baseUrl":"https://acme.test/v1","apiKey":"$ACME_KEY","models":[{"id":"acme-1"}]}}}"#,
    )
    .unwrap();
    let handles = build_catalogue_handles(tmp.path());
    let ModelListingOutcome::Listed(listing) = handles.list_models.list() else {
        panic!("a valid models.json lists");
    };
    assert!(
        listing
            .models
            .iter()
            .any(|m| m.entry.reference().qualified_id() == "acme/acme-1"),
        "{:?}",
        listing
            .models
            .iter()
            .map(|m| m.entry.reference().qualified_id())
            .collect::<Vec<_>>()
    );
}
