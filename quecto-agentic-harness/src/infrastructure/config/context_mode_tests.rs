//! #2403: the context mode switch, its marks and their validation.

use crate::domain::conversation::ContextMode;
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

fn watermark(high: usize, low: usize) -> ContextMode {
    ContextMode::Watermark(Watermark::new(high, low).unwrap())
}

#[test]
fn unset_the_mode_is_the_default_pruning() {
    let defaults = Config::default().agents.defaults;
    assert_eq!(defaults.context_mode(), ContextMode::Default);
    let defaults = load(r#"{"context_mode":"default"}"#)
        .unwrap()
        .agents
        .defaults;
    assert_eq!(defaults.context_mode(), ContextMode::Default);
}

#[test]
fn watermark_mode_cuts_at_256k_down_to_70k_unless_configured() {
    let defaults = load(r#"{"context_mode":"watermark"}"#)
        .unwrap()
        .agents
        .defaults;
    assert_eq!(defaults.context_mode(), watermark(256_000, 70_000));
    let defaults = load(
        r#"{"context_mode":"watermark","context_high_tokens":120000,"context_low_tokens":40000}"#,
    )
    .unwrap()
    .agents
    .defaults;
    assert_eq!(defaults.context_mode(), watermark(120_000, 40_000));
}

#[test]
fn the_environment_switches_the_mode_and_sets_the_marks() {
    let config = load_env(&[
        ("QUECTO_CONTEXT_MODE", "watermark"),
        ("QUECTO_CONTEXT_HIGH_TOKENS", "100000"),
        ("QUECTO_CONTEXT_LOW_TOKENS", "30000"),
    ])
    .unwrap();
    assert_eq!(
        config.agents.defaults.context_mode(),
        watermark(100_000, 30_000)
    );
    let config = load_env(&[("QUECTO_CONTEXT_MODE", "default")]).unwrap();
    assert_eq!(config.agents.defaults.context_mode(), ContextMode::Default);
}

/// Each refusal names what is wrong, at load, from the file or the
/// environment.
#[test]
fn invalid_values_are_refused_at_load_with_a_clear_message() {
    let refused = |result: Result<Config, ConfigError>, needle: &str| {
        let error = result.expect_err("refused").to_string();
        assert!(error.contains(needle), "{needle:?} in {error}");
    };
    refused(load(r#"{"context_mode":"watermarks"}"#), "context_mode");
    refused(load(r#"{"context_mode":"watermarks"}"#), "\"watermark\"");
    refused(
        load(
            r#"{"context_mode":"watermark","context_high_tokens":50000,"context_low_tokens":50000}"#,
        ),
        "below the high mark",
    );
    refused(
        load(r#"{"context_mode":"watermark","context_low_tokens":0}"#),
        "context_low_tokens",
    );
    refused(
        load(r#"{"context_mode":"watermark","context_high_tokens":0}"#),
        "context_high_tokens",
    );
    refused(
        load(
            r#"{"context_mode":"watermark","context_high_tokens":100000,"context_low_tokens":95000}"#,
        ),
        "too close",
    );
    refused(
        load_env(&[("QUECTO_CONTEXT_MODE", "fancy")]),
        "context_mode",
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

/// Unset keys are not written back, so a saved configuration is unchanged.
#[test]
fn unset_keys_are_not_serialized() {
    let value = serde_json::to_value(&Config::default().agents.defaults).unwrap();
    for key in ["context_mode", "context_high_tokens", "context_low_tokens"] {
        assert!(value.get(key).is_none(), "{key} in {value}");
    }
    let defaults = load(r#"{"context_mode":"watermark","context_high_tokens":120000}"#)
        .unwrap()
        .agents
        .defaults;
    let value = serde_json::to_value(&defaults).unwrap();
    assert_eq!(value["context_mode"], "watermark");
    assert_eq!(value["context_high_tokens"], 120_000);
}
