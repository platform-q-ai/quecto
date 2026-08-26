//! Tests for the effective-catalogue interface bridge (issue #1572).

use super::*;

#[test]
fn resolve_and_publish_for_publishes_one_generation_per_call() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, resolved) = resolve_and_publish_for(tmp.path());
    assert!(resolved.source_errors.is_empty());
    let first = store.current().generation();
    assert!(first >= 1);
    let (_, resolved_again) = resolve_and_publish_for(tmp.path());
    assert_eq!(resolved_again.snapshot.generation(), first + 1);
    // Same base_dir shares one store.
    assert_eq!(
        snapshot_store_for(tmp.path()).current().generation(),
        first + 1
    );
}

#[test]
fn unqualified_model_ids_have_no_limits() {
    let tmp = tempfile::tempdir().unwrap();
    let (cap, window) = model_limits_from_base_dir(tmp.path(), "not-qualified");
    assert_eq!(cap, None);
    assert_eq!(window, None);
}

#[test]
fn model_limits_from_base_dir_reads_output_cap_from_models_json() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"fireworks":{"api":"openai-completions","baseUrl":"https://e.example/v1","apiKey":"k","models":[{"id":"qwen3p7-plus","maxTokens":65536}]}}}"#,
    )
    .unwrap();

    let (cap, window) = model_limits_from_base_dir(tmp.path(), "fireworks/qwen3p7-plus");
    assert_eq!(cap, Some(65_536));
    assert_eq!(window, None, "no declared window must not clamp");
}

#[test]
fn model_limits_from_base_dir_reads_context_window_from_models_json() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"fireworks":{"api":"openai-completions","baseUrl":"https://e.example/v1","apiKey":"k","models":[{"id":"small-window","contextWindow":32768},{"id":"no-window"}]}}}"#,
    )
    .unwrap();

    assert_eq!(
        model_limits_from_base_dir(tmp.path(), "fireworks/small-window").1,
        Some(32_768)
    );
    assert_eq!(
        model_limits_from_base_dir(tmp.path(), "fireworks/no-window").1,
        None,
        "a listed model without a declared window must not clamp"
    );
}

#[test]
fn model_limits_survive_a_malformed_models_json_via_the_builtin_layer() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("models.json"), "not json").unwrap();
    // Malformed-source isolation: the built-in layer still resolves, so a
    // declared builtin window keeps clamping.
    assert_eq!(
        model_limits_from_base_dir(tmp.path(), "anthropic-api/claude-sonnet-5").1,
        Some(1_000_000)
    );
}
