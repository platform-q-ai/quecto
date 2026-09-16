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
    let sources = loaded.sources();
    let builtin: Vec<_> = sources
        .iter()
        .filter(|s| s.layer() == SourceLayer::BuiltIn)
        .collect();
    let user: Vec<_> = sources
        .iter()
        .filter(|s| s.layer() == SourceLayer::UserDefined)
        .collect();
    assert!(!builtin.is_empty() && !user.is_empty());
    assert!(builtin.iter().all(|s| s.load().is_ok()));
    assert!(user.iter().all(|s| s.load().is_err()));
}

#[test]
fn credential_status_is_a_per_record_verdict_driven_by_the_credential() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{
            "keyed":{"api":"openai-completions","apiKey":"sk-keyed","models":[{"id":"m"}]},
            "unset":{"api":"openai-completions","apiKey":"$QUECTO_CONTRACT_UNSET_KEY","models":[{"id":"m"}]},
            "bare":{"api":"openai-completions","models":[{"id":"m"}]}
        }}"#,
    )
    .unwrap();
    let loaded = under_test(tmp.path()).load();
    let user_entries: Vec<_> = loaded
        .sources()
        .iter()
        .filter(|s| s.layer() == SourceLayer::UserDefined)
        .flat_map(|s| s.load().unwrap().entries)
        .collect();
    let find = |id: &str| {
        user_entries
            .iter()
            .find(|e| e.reference().qualified_id() == id)
            .unwrap_or_else(|| panic!("{id} loads"))
    };
    let credentials = loaded.credentials();
    assert!(credentials.credential_available(find("keyed/m")));
    assert!(
        !credentials.credential_available(find("unset/m")),
        "an unresolved $ENV reference is no credential"
    );
    assert!(!credentials.credential_available(find("bare/m")));
}
