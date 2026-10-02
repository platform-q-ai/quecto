//! #2403/#2414: the watermark marks, plain settings with environment
//! overrides, and their validation.

use crate::domain::conversation::watermark::Watermark;
use crate::infrastructure::config::{Config, ConfigError};
use std::collections::HashMap;

fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn load(defaults: &str) -> Result<Config, ConfigError> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(&path, format!(r#"{{"agents":{{"defaults":{defaults}}}}}"#)).unwrap();
    Config::load(path.to_str().unwrap())
}

fn load_env(pairs: &[(&str, &str)]) -> Result<Config, ConfigError> {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.json");
    Config::load_with_env(missing.to_str().unwrap(), &env(pairs))
}

fn marks(high: usize, low: usize) -> Watermark {
    Watermark::new(high, low).unwrap()
}

#[test]
fn the_marks_cut_at_256k_down_to_70k_unless_configured() {
    assert_eq!(
        Config::default().agents.defaults.context_marks(),
        marks(256_000, 70_000)
    );
    assert_eq!(
        load("{}").unwrap().agents.defaults.context_marks(),
        marks(256_000, 70_000)
    );
    let defaults = load(r#"{"context_high_tokens":120000,"context_low_tokens":40000}"#)
        .unwrap()
        .agents
        .defaults;
    assert_eq!(defaults.context_marks(), marks(120_000, 40_000));
}

#[test]
fn the_environment_sets_the_marks() {
    let config = load_env(&[
        ("QUECTO_CONTEXT_HIGH_TOKENS", "100000"),
        ("QUECTO_CONTEXT_LOW_TOKENS", "30000"),
    ])
    .unwrap();
    assert_eq!(
        config.agents.defaults.context_marks(),
        marks(100_000, 30_000)
    );
    let config = load_env(&[("QUECTO_CONTEXT_HIGH_TOKENS", "300000")]).unwrap();
    assert_eq!(
        config.agents.defaults.context_marks(),
        marks(300_000, 70_000),
        "an unset mark is the owner's"
    );
}

/// Each refusal names what is wrong, at load, from the file or the
/// environment.
#[test]
fn invalid_marks_are_refused_at_load_with_a_clear_message() {
    let refused = |result: Result<Config, ConfigError>, needle: &str| {
        let error = result.expect_err("refused").to_string();
        assert!(error.contains(needle), "{needle:?} in {error}");
    };
    refused(
        load(r#"{"context_high_tokens":50000,"context_low_tokens":50000}"#),
        "below the high mark",
    );
    refused(load(r#"{"context_low_tokens":0}"#), "context_low_tokens");
    refused(load(r#"{"context_high_tokens":0}"#), "context_high_tokens");
    refused(
        load(r#"{"context_high_tokens":100000,"context_low_tokens":95000}"#),
        "too close",
    );
    refused(
        load_env(&[("QUECTO_CONTEXT_HIGH_TOKENS", "lots")]),
        "QUECTO_CONTEXT_HIGH_TOKENS",
    );
    refused(
        load_env(&[("QUECTO_CONTEXT_LOW_TOKENS", "-1")]),
        "QUECTO_CONTEXT_LOW_TOKENS",
    );
    refused(
        load_env(&[
            ("QUECTO_CONTEXT_HIGH_TOKENS", "60000"),
            ("QUECTO_CONTEXT_LOW_TOKENS", "70000"),
        ]),
        "below the high mark",
    );
}

/// Unset keys are not written back, so a saved configuration is unchanged;
/// nor is a removed key ever written.
#[test]
fn unset_keys_are_not_serialized() {
    let value = serde_json::to_value(&Config::default().agents.defaults).unwrap();
    for key in ["context_mode", "context_high_tokens", "context_low_tokens"] {
        assert!(value.get(key).is_none(), "{key} in {value}");
    }
    let defaults = load(r#"{"context_high_tokens":120000}"#)
        .unwrap()
        .agents
        .defaults;
    let value = serde_json::to_value(&defaults).unwrap();
    assert_eq!(value["context_high_tokens"], 120_000);
    assert!(value.get("context_low_tokens").is_none(), "{value}");
}

/// Review nit: each environment override beats the file.
#[test]
fn the_environment_overrides_the_file_for_each_mark() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(
        &path,
        r#"{"agents":{"defaults":{"context_high_tokens":120000,"context_low_tokens":40000}}}"#,
    )
    .unwrap();
    let config = Config::load_with_env(
        path.to_str().unwrap(),
        &env(&[
            ("QUECTO_CONTEXT_HIGH_TOKENS", "90000"),
            ("QUECTO_CONTEXT_LOW_TOKENS", "20000"),
        ]),
    )
    .unwrap();
    assert_eq!(
        config.agents.defaults.context_marks(),
        marks(90_000, 20_000)
    );
}
