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
