#[test]
fn catalogue_limit_source_preserves_legacy_base_dir_limit_lookup() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"fireworks":{"api":"openai-completions","baseUrl":"https://api.fireworks.ai/inference/v1","auth":{"mode":"apiKey","apiKey":"k"},"models":[{"id":"small","contextWindow":2048,"maxTokens":512}]}}}"#,
    )
    .unwrap();

    assert_eq!(
        super::model_limits_from_base_dir(tmp.path(), "fireworks/small"),
        (Some(512), Some(2048))
    );
    assert_eq!(
        super::model_limits_from_base_dir(tmp.path(), "not-qualified"),
        (None, None)
    );
}

#[test]
fn catalogue_limit_source_falls_back_to_builtin_limits_when_models_json_is_malformed() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(tmp.path().join("models.json"), "{ not json").unwrap();

    let builtin = crate::infrastructure::model_registry::ModelRegistry::builtin();
    let sample = builtin
        .models()
        .iter()
        .find(|model| model.context_window_explicit)
        .expect("builtin registry should pin at least one explicit context window");
    let qualified = format!("{}/{}", sample.provider, sample.id);

    assert_eq!(
        super::model_limits_from_base_dir(tmp.path(), &qualified),
        (
            builtin.max_tokens_for(&qualified),
            builtin.context_window_for(&qualified)
        )
    );
    assert!(builtin.context_window_for(&qualified).is_some());
}
