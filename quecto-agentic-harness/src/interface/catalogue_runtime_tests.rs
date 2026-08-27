//! Tests for the interface runtime-composition wiring (issue #1573): the real
//! factory, sources, and stores behind `compose_and_publish_runtime` /
//! `select_model`, driven with on-disk models.json fixtures (no network).

use super::*;
use crate::application::provider_runtime::SelectionError;
use crate::domain::catalogue::UnavailableReason;

fn write_models_json(dir: &Path, body: &str) {
    std::fs::write(dir.join("models.json"), body).unwrap();
}

const RUNNABLE_AND_KEYLESS: &str = r#"{"providers":{
    "wired":{"api":"openai-completions","apiKey":"sk-wired","baseUrl":"https://api.example.test/v1",
        "models":[{"id":"wired-model","name":"Wired Model"}]},
    "keyless":{"api":"openai-completions",
        "models":[{"id":"keyless-model","name":"Keyless Model"}]}
}}"#;

fn compose(dir: &Path) -> Result<Arc<CatalogueRuntimeSnapshot>, RuntimeCompositionError> {
    compose_and_publish_runtime(&Config::default(), dir, &reqwest::Client::new())
}

#[test]
fn failed_composition_publishes_no_runtime_and_selection_reports_no_runtime() {
    let tmp = tempfile::tempdir().unwrap();
    // No credentials anywhere: the factory fails, nothing is published.
    let error = compose(tmp.path()).expect_err("composition fails without providers");
    assert!(error.error.contains("no LLM providers configured"));
    assert!(error.retained.is_none());
    assert!(runtime_store_for(tmp.path()).current().is_none());
    assert_eq!(
        select_model(tmp.path(), "openai-api/gpt-5").expect_err("no runtime yet"),
        SelectionError::NoRuntime
    );
}

#[test]
fn composed_runtime_and_catalogue_share_one_generation_and_selection_matches() {
    let tmp = tempfile::tempdir().unwrap();
    write_models_json(tmp.path(), RUNNABLE_AND_KEYLESS);
    let snapshot = compose(tmp.path()).expect("composition succeeds");
    assert_eq!(
        snapshot.generation(),
        snapshot_store_for(tmp.path()).current().generation(),
        "runtime and catalogue stores publish the same generation"
    );
    let selection = select_model(tmp.path(), "wired/wired-model").expect("runnable model selects");
    assert_eq!(selection.entry.provider.id.as_str(), "wired");
    assert_eq!(selection.generation, snapshot.generation());

    // A failed re-composition retains this generation.
    std::fs::remove_file(tmp.path().join("models.json")).unwrap();
    let error = compose(tmp.path()).expect_err("re-composition fails");
    let retained = error.retained.expect("previous runtime retained");
    assert_eq!(retained.generation(), snapshot.generation());
    assert_eq!(
        runtime_store_for(tmp.path())
            .current()
            .expect("still published")
            .generation(),
        snapshot.generation()
    );
}

#[test]
fn selection_returns_structured_reasons_for_unknown_and_unrunnable_models() {
    let tmp = tempfile::tempdir().unwrap();
    write_models_json(tmp.path(), RUNNABLE_AND_KEYLESS);
    compose(tmp.path()).expect("composition succeeds");
    assert_eq!(
        select_model(tmp.path(), "wired/no-such-model").expect_err("unknown model"),
        SelectionError::UnknownModel {
            reference: "wired/no-such-model".to_string()
        }
    );
    assert_eq!(
        select_model(tmp.path(), "not a qualified ref").expect_err("unparsable reference"),
        SelectionError::UnknownModel {
            reference: "not a qualified ref".to_string()
        }
    );
    match select_model(tmp.path(), "keyless/keyless-model").expect_err("unrunnable model") {
        SelectionError::NotRunnable { reasons, .. } => {
            assert!(reasons.contains(&UnavailableReason::MissingCredential));
        }
        other => panic!("expected NotRunnable, got {other:?}"),
    }
}
