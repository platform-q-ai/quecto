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
fn a_bare_model_id_resolves_when_every_provider_serving_it_agrees() {
    let tmp = tempfile::tempdir().unwrap();
    let _ = quecto::composition::catalogue::list_models_wire_for(tmp.path());
    let source = under_test(tmp.path());
    // gpt-5.6-sol is built in under openai-api and openai-oauth with one scale.
    assert_eq!(
        source.effort_vocabulary("gpt-5.6-sol"),
        Some(vec![
            quecto::domain::provider::EffortLevel::None,
            Low,
            Medium,
            High,
            XHigh
        ])
    );
    // gpt-5.5: openai-api (Chat Completions, no reasoning) offers nothing,
    // openai-oauth (Responses) offers the scale — ambiguous as a bare id.
    assert_eq!(source.effort_vocabulary("gpt-5.5"), None);
    assert_eq!(source.effort_vocabulary("no-such-model"), None);
}

#[test]
fn a_bare_model_id_served_with_different_vocabularies_is_ambiguous() {
    let tmp = tempfile::tempdir().unwrap();
    // A user provider also serves the built-in id `gpt-5.6-sol`, declaring
    // no reasoning: openai-api/openai-oauth offer the OpenAI scale, this
    // one offers nothing. Nothing can be affirmed for the bare id.
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"gateway":{"api":"openai-completions","baseUrl":"https://gw.test/v1","apiKey":"$GW","models":[{"id":"gpt-5.6-sol"}]}}}"#,
    )
    .unwrap();
    let _ = quecto::composition::catalogue::list_models_wire_for(tmp.path());
    let source = under_test(tmp.path());
    assert_eq!(source.effort_vocabulary("gpt-5.6-sol"), None);
    assert_eq!(
        source.effort_vocabulary("openai-api/gpt-5.6-sol"),
        Some(vec![
            quecto::domain::provider::EffortLevel::None,
            Low,
            Medium,
            High,
            XHigh
        ]),
        "the qualified id is unambiguous"
    );
}
