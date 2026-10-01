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

/// Review nit: each of the three environment overrides beats the file.
#[test]
fn the_environment_overrides_the_file_for_each_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(
        &path,
        r#"{"agents":{"defaults":{"context_mode":"default","context_high_tokens":120000,"context_low_tokens":40000}}}"#,
    )
    .unwrap();
    let config = Config::load_with_env(
        path.to_str().unwrap(),
        &env(&[
            ("QUECTO_CONTEXT_MODE", "watermark"),
            ("QUECTO_CONTEXT_HIGH_TOKENS", "90000"),
            ("QUECTO_CONTEXT_LOW_TOKENS", "20000"),
        ]),
    )
    .unwrap();
    assert_eq!(
        config.agents.defaults.context_mode(),
        watermark(90_000, 20_000)
    );
}

/// A member's configuration, loaded from `defaults` and `pairs`, then
/// given what its parent handed down with `--inherited-context-mode`.
fn member(defaults: &str, pairs: &[(&str, &str)], inherited: &str) -> Result<Config, ConfigError> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(&path, format!(r#"{{"agents":{{"defaults":{defaults}}}}}"#)).unwrap();
    let mut config = Config::load_with_env(path.to_str().unwrap(), &env(pairs))?;
    config.agents.defaults.inherit_context_mode(inherited)?;
    Ok(config)
}

const PARENT: &str = "watermark:256000:70000";

/// Final review M1/M3: for each of the mode, the high and the low mark, the
/// member's own configuration or environment wins, then what the parent
/// handed down, then the default.
#[test]
fn a_member_inherits_only_what_its_own_configuration_leaves_unset() {
    let mode = |config: Config| config.agents.defaults.context_mode();
    assert_eq!(
        mode(member("{}", &[], PARENT).unwrap()),
        watermark(256_000, 70_000)
    );
    assert_eq!(
        mode(
            member(
                r#"{"context_high_tokens":300000,"context_low_tokens":100000}"#,
                &[],
                PARENT
            )
            .unwrap()
        ),
        watermark(300_000, 100_000),
        "own marks win"
    );
    assert_eq!(
        mode(member("{}", &[("QUECTO_CONTEXT_HIGH_TOKENS", "300000")], PARENT).unwrap()),
        watermark(300_000, 70_000),
        "an own env mark wins, the other is inherited"
    );
    assert_eq!(
        mode(member(r#"{"context_mode":"default"}"#, &[], PARENT).unwrap()),
        ContextMode::Default,
        "an own mode wins"
    );
    assert_eq!(
        mode(member("{}", &[("QUECTO_CONTEXT_MODE", "default")], PARENT).unwrap()),
        ContextMode::Default,
        "an own env mode wins"
    );
    assert_eq!(
        mode(
            member(
                r#"{"context_mode":"watermark"}"#,
                &[],
                "watermark:120000:40000"
            )
            .unwrap()
        ),
        watermark(120_000, 40_000),
        "an own mode without marks takes the parent's marks"
    );
    assert_eq!(
        mode(member("{}", &[], "default").unwrap()),
        ContextMode::Default
    );
    let error = member("{}", &[], "watermark:lots")
        .expect_err("refused")
        .to_string();
    assert!(error.contains("--inherited-context-mode"), "{error}");
    let error = member("{}", &[], "watermark:100000:95000")
        .expect_err("refused")
        .to_string();
    assert!(error.contains("too close"), "{error}");
}

/// The value a parent hands a member round-trips to the same mode, and
/// the environment carries none (the CLI argument is the only channel).
#[test]
fn the_inherited_value_round_trips_and_no_environment_carries_it() {
    use super::inherited_value;
    for mode in [ContextMode::Default, watermark(120_000, 40_000)] {
        let config = member("{}", &[], &inherited_value(mode)).unwrap();
        assert_eq!(config.agents.defaults.context_mode(), mode);
    }
    let config = load_env(&[("QUECTO_INHERITED_CONTEXT_MODE", PARENT)]).unwrap();
    assert_eq!(config.agents.defaults.context_mode(), ContextMode::Default);
}

/// Final review L4: where the effective mode came from.
#[test]
fn the_mode_names_where_it_came_from() {
    use super::ContextModeSource as S;
    let source = |config: Config| config.agents.defaults.context_mode_source();
    assert_eq!(source(member("{}", &[], PARENT).unwrap()), S::Inherited);
    assert_eq!(
        source(member(r#"{"context_mode":"watermark"}"#, &[], PARENT).unwrap()),
        S::OwnConfiguration
    );
    assert_eq!(
        source(member("{}", &[("QUECTO_CONTEXT_MODE", "watermark")], PARENT).unwrap()),
        S::OwnEnvironment
    );
    assert_eq!(source(load_env(&[]).unwrap()), S::Default);
}
