//! Contract for the `ConfigValidator` port (#2024): resolution rejects the
//! retired keys and resolves file-relative references against the
//! document's own file; validation applies every load-time rule the
//! harness applies to a config file, and unknown keys are tolerated.
use std::sync::Arc;
use tempfile::TempDir;

use quecto::application::configuration::ports::ConfigValidator;
use quecto::infrastructure::config::mapping::ConfigValidatorAdapter;

fn under_test() -> Arc<dyn ConfigValidator> {
    Arc::new(ConfigValidatorAdapter)
}

#[test]
fn validation_accepts_partial_documents_and_names_what_is_wrong() {
    let validator = under_test();
    assert!(validator.validate(&serde_json::json!({})).is_ok());
    assert!(
        validator
            .validate(&serde_json::json!({"custom": true, "agents": {"defaults": {"model": "m"}}}))
            .is_ok()
    );
    let error = validator
        .validate(&serde_json::json!({"agents": {"defaults": {"effort": "bogus"}}}))
        .unwrap_err();
    assert!(error.contains("invalid effort level"), "{error}");
    let error = validator
        .validate(&serde_json::json!({"container_configs": {"a": {"create": ["x"]}, "b": {"create": ["y"]}}}))
        .unwrap_err();
    assert!(error.contains("default"), "{error}");
}

#[test]
fn resolution_rejects_retired_keys_and_resolves_references_against_the_file() {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("step.json"),
        r#"{"key":"s","label":"S","phase":"green","guidance":"g"}"#,
    )
    .unwrap();
    let path = dir.path().join("config.json");
    let validator = under_test();
    let error = validator
        .resolve(serde_json::json!({"container_scripts": {}}), &path)
        .unwrap_err();
    assert!(error.contains("container_scripts"), "{error}");
    let resolved = validator
        .resolve(
            serde_json::json!({"workflow":{"templates":[{"id":"t","label":"T","description":"d","steps":["step"]}]}}),
            &path,
        )
        .unwrap();
    assert_eq!(resolved["workflow"]["templates"][0]["steps"][0]["key"], "s");
    let elsewhere = dir.path().join("elsewhere").join("config.json");
    assert!(
        validator
            .resolve(
                serde_json::json!({"workflow":{"templates":[{"id":"t","label":"T","description":"d","steps":["step"]}]}}),
                &elsewhere,
            )
            .is_err(),
        "references resolve against the document's own directory"
    );
}
