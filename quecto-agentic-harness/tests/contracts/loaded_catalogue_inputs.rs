//! Contract for `LoadedCatalogueInputs` (#1845): one load's sources come
//! in application precedence order with the built-in layer first, a
//! malformed `models.json` fails only its own layer, and the credential
//! port answers yes/no — no secret value is reachable through the inputs.
use std::sync::Arc;

use quecto::application::catalogue::ports::CatalogueInputsLoader;
use quecto::domain::catalogue::SourceLayer;
use quecto::infrastructure::catalogue_inputs::FileCatalogueInputs;

fn under_test(dir: &std::path::Path) -> Arc<dyn CatalogueInputsLoader> {
    Arc::new(FileCatalogueInputs::new(dir))
}

#[test]
fn sources_come_in_precedence_order_starting_with_the_builtin_layer() {
    let tmp = tempfile::tempdir().unwrap();
    let loaded = under_test(tmp.path()).load();
    let layers: Vec<SourceLayer> = loaded.sources().iter().map(|s| s.layer()).collect();
    assert_eq!(layers.first(), Some(&SourceLayer::BuiltIn));
    assert!(layers.windows(2).all(|w| w[0] <= w[1]), "{layers:?}");
}

#[test]
fn a_malformed_models_json_fails_its_own_layer_only() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("models.json"), "{ not json").unwrap();
    let loaded = under_test(tmp.path()).load();
    let builtin_ok = loaded
        .sources()
        .iter()
        .filter(|s| s.layer() == SourceLayer::BuiltIn)
        .all(|s| s.load().is_ok());
    let user_err = loaded
        .sources()
        .iter()
        .filter(|s| s.layer() == SourceLayer::UserDefined)
        .all(|s| s.load().is_err());
    assert!(builtin_ok && user_err);
}

#[test]
fn credential_status_is_a_verdict_never_a_value() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"acme":{"api":"openai-completions","baseUrl":"https://acme.test/v1","apiKey":"sk-literal-secret","models":[{"id":"acme-1"}]}}}"#,
    )
    .unwrap();
    let loaded = under_test(tmp.path()).load();
    let user_entries: Vec<_> = loaded
        .sources()
        .iter()
        .filter(|s| s.layer() == SourceLayer::UserDefined)
        .flat_map(|s| s.load().unwrap().entries)
        .collect();
    let acme = user_entries
        .iter()
        .find(|e| e.reference().qualified_id() == "acme/acme-1")
        .expect("user model loads");
    // The verdict is observable; the value is not (a literal key is a
    // rejected override, so it counts as unavailable — either way only a
    // bool leaves the port).
    let _verdict: bool = loaded.credentials().credential_available(acme);
    assert!(!format!("{acme:?}").contains("sk-literal-secret"));
}
