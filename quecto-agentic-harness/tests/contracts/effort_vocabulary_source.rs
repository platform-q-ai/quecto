//! Contract for the `EffortVocabularySource` port (#1848, #1996): the
//! vocabulary is the published snapshot's per-model `effort_levels`, in
//! order; a model the snapshot does not hold has none (`None`), and a known
//! model that declares no reasoning has an empty vocabulary. The adapter
//! reads the current generation on every call, so a republish is honoured.
use std::sync::Arc;

use quecto::application::catalogue::ports::EffortVocabularySource;
use quecto::domain::provider::EffortLevel::{High, Low, Medium, XHigh};
use quecto::infrastructure::catalogue_registry::PublishedEffortVocabulary;

fn under_test(dir: &std::path::Path) -> Arc<dyn EffortVocabularySource> {
    Arc::new(PublishedEffortVocabulary::for_base_dir(dir))
}

#[test]
fn builtin_xai_models_carry_their_documented_scales_after_publish() {
    let tmp = tempfile::tempdir().unwrap();
    let source = under_test(tmp.path());
    assert_eq!(
        source.effort_vocabulary("xai/grok-4.6"),
        None,
        "nothing published yet: the model is unknown"
    );
    let _ = quecto::composition::catalogue::list_models_wire_for(tmp.path());
    assert_eq!(
        source.effort_vocabulary("xai/grok-4.6"),
        Some(vec![Low, Medium, High, XHigh])
    );
    assert_eq!(
        source.effort_vocabulary("xai/grok-4.5"),
        Some(vec![Low, Medium, High])
    );
    assert_eq!(source.effort_vocabulary("openrouter/never-listed"), None);
    assert_eq!(source.effort_vocabulary("not a reference"), None);
}

#[test]
fn a_user_record_declares_its_control_with_the_reasoning_flag() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"fireworks":{"api":"openai-completions","baseUrl":"https://api.fireworks.ai/inference/v1","apiKey":"$FW","models":[
            {"id":"accounts/fireworks/models/glm-5p3","reasoning":true},
            {"id":"accounts/fireworks/models/plain"}
        ]}}}"#,
    )
    .unwrap();
    let _ = quecto::composition::catalogue::list_models_wire_for(tmp.path());
    let source = under_test(tmp.path());
    assert_eq!(
        source.effort_vocabulary("fireworks/accounts/fireworks/models/glm-5p3"),
        Some(vec![Low, Medium, High])
    );
    assert_eq!(
        source.effort_vocabulary("fireworks/accounts/fireworks/models/plain"),
        Some(vec![]),
        "known, but no reasoning declared: no effort control"
    );
}

#[test]
fn a_bare_model_id_resolves_across_the_runnable_providers_serving_it() {
    let tmp = tempfile::tempdir().unwrap();
    // `a` and `b` are runnable (a key is declared) and agree; `c` is not
    // runnable (no credential) and would disagree, so it does not count —
    // the router can only serve a bare id from a configured provider.
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{
            "a":{"api":"openai-completions","baseUrl":"https://a.test/v1","apiKey":"sk-a","models":[{"id":"shared","reasoning":true}]},
            "b":{"api":"openai-completions","baseUrl":"https://b.test/v1","apiKey":"sk-b","models":[{"id":"shared","reasoning":true}]},
            "c":{"api":"openai-completions","models":[{"id":"shared"}]}
        }}"#,
    )
    .unwrap();
    let _ = quecto::composition::catalogue::list_models_wire_for(tmp.path());
    let source = under_test(tmp.path());
    assert_eq!(
        source.effort_vocabulary("shared"),
        Some(vec![Low, Medium, High])
    );
    // No runnable provider serves the built-in bare id in this directory
    // (no OpenAI credential): nothing is known.
    assert_eq!(source.effort_vocabulary("gpt-5.6-sol"), None);
    assert_eq!(source.effort_vocabulary("no-such-model"), None);
}

#[test]
fn a_bare_model_id_served_with_different_vocabularies_is_ambiguous() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{
            "a":{"api":"openai-completions","baseUrl":"https://a.test/v1","apiKey":"sk-a","models":[{"id":"shared","reasoning":true}]},
            "b":{"api":"openai-completions","baseUrl":"https://b.test/v1","apiKey":"sk-b","models":[{"id":"shared"}]}
        }}"#,
    )
    .unwrap();
    let _ = quecto::composition::catalogue::list_models_wire_for(tmp.path());
    let source = under_test(tmp.path());
    assert_eq!(
        source.effort_vocabulary("shared"),
        None,
        "runnable providers disagree"
    );
    assert_eq!(
        source.effort_vocabulary("a/shared"),
        Some(vec![Low, Medium, High]),
        "the qualified id is unambiguous"
    );
}
