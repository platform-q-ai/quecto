//! Contract for the `ModelRefusalSink` port (#2435): the first refusal of a
//! model is recorded (`true`), a repeat is not new (`false`), and from then
//! on the base directory's listing shows the model as not runnable with the
//! provider's reason, while its siblings and its other auth mode stay as
//! they were.
use std::sync::Arc;

use quecto::application::catalogue::ports::ModelRefusalSink;
use quecto::domain::catalogue::ModelRef;
use quecto::infrastructure::catalogue_registry::snapshot_store_for;

const HELD: std::time::Duration = std::time::Duration::from_secs(3600);

const REASON: &str =
    "The 'gpt-6-luna' model is not supported when using Codex with a ChatGPT account.";

fn listed<'a>(wire: &'a serde_json::Value, model: &str) -> &'a serde_json::Value {
    wire["models"]
        .as_array()
        .expect("a listing")
        .iter()
        .find(|m| m["model"] == model)
        .unwrap_or_else(|| panic!("{model} listed"))
}

#[test]
fn a_recorded_refusal_is_new_once_and_lists_the_model_as_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let sink: Arc<dyn ModelRefusalSink> = Arc::new(snapshot_store_for(tmp.path()));
    let refused = ModelRef::parse_qualified("openai-oauth/gpt-6-luna").unwrap();
    assert!(
        sink.record_refusal(&refused, REASON, HELD),
        "the first is new"
    );
    assert!(
        !sink.record_refusal(&refused, "again", HELD),
        "a repeat is not"
    );

    let wire = quecto::composition::catalogue::list_models_wire_for(tmp.path());
    let entry = listed(&wire, "openai-oauth/gpt-6-luna");
    assert_eq!(entry["configured"], false);
    assert!(
        entry["unavailable"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!(format!("refused-for-account: {REASON}"))),
        "{entry}"
    );
    for other in ["openai-oauth/gpt-6.1-sol", "openai-api/gpt-6-luna"] {
        let reasons = listed(&wire, other)["unavailable"]
            .as_array()
            .unwrap()
            .clone();
        assert!(
            reasons
                .iter()
                .all(|r| !r.as_str().unwrap().starts_with("refused-for-account")),
            "{other} is not refused: {reasons:?}"
        );
    }
}
