//! Contract for the `EffortDefaultPersistence` port (#2024 S2): a record
//! lands `agents.defaults.effort` as the level's API string in exactly
//! the layer the scope names, leaving the model and every other key as
//! they were; the shared fixture is `model_default_persistence.rs`'s.
use std::sync::Arc;

use quecto::application::catalogue::ports::{DefaultScope, EffortDefaultPersistence};
use quecto::domain::provider::EffortLevel;

use super::model_default_persistence::{Layers, json, layers, writer};

fn under_test(layers: &Layers) -> Arc<dyn EffortDefaultPersistence> {
    Arc::new(writer(layers))
}

#[test]
fn a_local_record_writes_the_level_string_beside_an_existing_model() {
    let fx = layers(r#"{"agents":{"defaults":{"model":"g/m"}}}"#);
    let persisted = under_test(&fx)
        .persist_effort(DefaultScope::Local, EffortLevel::XHigh)
        .unwrap();
    assert_eq!(persisted.path, fx.overlay);
    assert_eq!(
        json(&fx.overlay),
        serde_json::json!({"agents":{"defaults":{"effort":"xhigh"}}})
    );
    assert_eq!(json(&fx.global)["agents"]["defaults"]["model"], "g/m");
}

#[test]
fn a_global_record_replaces_only_the_effort_line() {
    let fx = layers(
        "{\n  \"agents\": {\n    \"defaults\": {\n      \"model\": \"g/m\",\n      \"effort\": \"high\"\n    }\n  }\n}\n",
    );
    let before = std::fs::read_to_string(&fx.global).unwrap();
    let persisted = under_test(&fx)
        .persist_effort(DefaultScope::Global, EffortLevel::Low)
        .unwrap();
    assert_eq!(persisted.path, fx.global);
    assert_eq!(
        std::fs::read_to_string(&fx.global).unwrap(),
        before.replace("\"high\"", "\"low\"")
    );
}
