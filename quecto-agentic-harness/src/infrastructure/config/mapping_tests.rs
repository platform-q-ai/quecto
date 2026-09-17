use super::*;
use serde_json::json;
use tempfile::TempDir;

#[test]
fn validation_applies_every_load_time_rule() {
    let validator = ConfigValidatorAdapter;
    assert!(validator.validate(&json!({})).is_ok());
    assert!(
        validator
            .validate(&json!({"unknown": 1, "agents": {"defaults": {"model": "m"}}}))
            .is_ok()
    );
    let effort = validator
        .validate(&json!({"agents": {"defaults": {"effort": "bogus"}}}))
        .unwrap_err();
    assert!(effort.contains("invalid effort level"), "{effort}");
    let containers = validator
        .validate(&json!({"container_configs": {"a": {"create": ["x"]}}}))
        .unwrap_err();
    assert!(containers.contains("default"), "{containers}");
    let shape = validator.validate(&json!({"agents": "junk"})).unwrap_err();
    assert!(shape.contains("failed to parse config"), "{shape}");
}

#[test]
fn a_layer_may_add_a_non_default_container_config_but_keeps_every_other_rule() {
    let validator = ConfigValidatorAdapter;
    let layer = json!({"container_configs": {"a": {"create": ["x"]}}});
    assert!(validator.validate_layer(&layer).is_ok());
    assert!(
        validator.validate(&layer).is_err(),
        "the merge still needs a default"
    );
    assert!(
        validator
            .validate_layer(&json!({"agents": {"defaults": {"effort": "bogus"}}}))
            .is_err()
    );
    assert!(
        validator
            .validate_layer(&json!({"agents": "junk"}))
            .is_err()
    );
    assert!(
        validator
            .validate_layer(&json!({"admission": {"groups": {}}}))
            .is_err()
    );
}

#[test]
fn resolution_rejects_retired_keys_and_resolves_step_references_against_the_file() {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("shared.json"),
        r#"{"key":"shared","label":"Shared","phase":"green","guidance":"g"}"#,
    )
    .unwrap();
    let path = dir.path().join("config.json");
    let validator = ConfigValidatorAdapter;
    let error = validator
        .resolve(json!({"container_scripts": {}}), &path)
        .unwrap_err();
    assert!(error.contains("container_scripts"), "{error}");
    let resolved = validator
        .resolve(
            json!({"workflow":{"templates":[{"id":"t","label":"T","description":"d","steps":["shared"]}]}}),
            &path,
        )
        .unwrap();
    assert_eq!(
        resolved["workflow"]["templates"][0]["steps"][0]["key"],
        "shared"
    );
    assert!(validator.validate(&resolved).is_ok());
}

#[test]
fn realize_applies_env_overrides_after_the_document_and_rejects_bad_ones() {
    let base = TempDir::new().unwrap();
    let env: HashMap<String, String> = [(
        "QUECTO_AGENTS_DEFAULTS_MODEL".to_string(),
        "env-model".to_string(),
    )]
    .into_iter()
    .collect();
    let config = realize_config(
        json!({"agents": {"defaults": {"model": "doc-model"}}}),
        &env,
        base.path(),
    )
    .unwrap();
    assert_eq!(config.agents.defaults.model, "env-model");
    assert!(
        config.admission_proposal().unwrap().is_none(),
        "no admission section keeps admission disabled"
    );

    let env: HashMap<String, String> = [(
        "QUECTO_AGENTS_DEFAULTS_EFFORT".to_string(),
        "bogus".to_string(),
    )]
    .into_iter()
    .collect();
    let error = realize_config(json!({}), &env, base.path()).unwrap_err();
    assert!(error.contains("invalid effort level"), "{error}");
    let error = realize_config(json!({"agents": 1}), &env, base.path()).unwrap_err();
    assert!(error.contains("failed to parse config"), "{error}");
}
