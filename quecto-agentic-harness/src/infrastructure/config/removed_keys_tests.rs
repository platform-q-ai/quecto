//! #2414: a configuration that still sets a key of the removed pruning
//! rules, or the removed context-mode switch, is refused at load, from the
//! file or the environment, with a message naming the key.

use crate::infrastructure::config::{Config, ConfigError};
use std::collections::HashMap;

/// Every removed `agents.defaults` key, with a value it used to take.
const REMOVED_KEYS: [(&str, &str); 6] = [
    ("context_mode", r#""watermark""#),
    ("context_collapse_after_tool_calls", "50"),
    // The pre-#1017 name of the tool dial.
    ("context_collapse_after_turns", "50"),
    ("context_collapse_after_messages", "50"),
    ("context_collapse_large_result_tokens", "2000"),
    ("context_collapse_large_result_after_turns", "2"),
];

/// The schema refuses exactly the keys the application's policy names.
#[test]
fn the_refused_keys_are_the_policys() {
    use crate::domain::conversation::removed_keys::REMOVED_DEFAULTS_KEYS;
    let keys: Vec<&str> = REMOVED_KEYS.iter().map(|(key, _)| *key).collect();
    assert_eq!(keys, REMOVED_DEFAULTS_KEYS);
}

/// Review H1: every removed key set is named, in one message, with what
/// removes it.
#[test]
fn every_removed_key_set_is_named_in_one_message() {
    let document = serde_json::json!({"agents": {"defaults": {
        "context_collapse_after_tool_calls": 100,
        "context_collapse_after_messages": 100,
    }}});
    let error = Config::from_document(document).unwrap_err().to_string();
    for key in [
        "context_collapse_after_tool_calls",
        "context_collapse_after_messages",
    ] {
        assert!(
            error.contains(&format!("quecto config unset agents.defaults.{key}")),
            "{key}: {error}"
        );
    }
}

/// Every removed environment override, with a value it used to take.
const REMOVED_ENV: [(&str, &str); 3] = [
    ("QUECTO_CONTEXT_MODE", "watermark"),
    ("QUECTO_CONTEXT_COLLAPSE_LARGE_RESULT_TOKENS", "2000"),
    ("QUECTO_CONTEXT_COLLAPSE_LARGE_RESULT_AFTER_TURNS", "2"),
];

fn document(key: &str, value: &str) -> serde_json::Value {
    let text = format!(r#"{{"agents":{{"defaults":{{"{key}":{value}}}}}}}"#);
    serde_json::from_str(&text).unwrap()
}

fn assert_refused(result: Result<Config, ConfigError>, key: &str, how: &str) {
    let error = match result {
        Ok(_) => panic!("{how}: a config setting {key} is refused"),
        Err(error) => error.to_string(),
    };
    assert!(
        error.contains(key) && error.contains("removed"),
        "{how}: the refusal names {key} and says it was removed: {error}"
    );
}

#[test]
fn a_config_setting_a_removed_key_is_refused_naming_it() {
    for (key, value) in REMOVED_KEYS {
        let document = document(key, value);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, document.to_string()).unwrap();
        assert_refused(Config::load(path.to_str().unwrap()), key, "the file");
        assert_refused(Config::from_document(document.clone()), key, "a document");
        assert_refused(
            Config::layer_from_document(document.clone()),
            key,
            "a layer",
        );
        assert_refused(
            Config::from_inherited_child_document(document),
            key,
            "a child's document",
        );
    }
}

/// Set at all is set: a removed key with a null value is refused too.
#[test]
fn a_removed_key_set_to_null_is_refused_too() {
    for (key, _) in REMOVED_KEYS {
        assert_refused(Config::from_document(document(key, "null")), key, "null");
    }
}

#[test]
fn a_removed_environment_override_is_refused_naming_it() {
    for (key, value) in REMOVED_ENV {
        let env = HashMap::from([(key.to_string(), value.to_string())]);
        assert_refused(
            Config::default().with_env_overrides(&env),
            key,
            "the environment",
        );
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.json");
        assert_refused(
            Config::load_with_env(missing.to_str().unwrap(), &env),
            key,
            "a load with the environment",
        );
    }
}

/// What stays is still a plain setting, from the file and the
/// environment: the marks, the ceilings and the ladder's pinned turns.
#[test]
fn the_kept_context_settings_still_load() {
    let document = serde_json::json!({"agents": {"defaults": {
        "context_high_tokens": 120_000,
        "context_low_tokens": 40_000,
        "max_context_tokens": 200_000,
        "swarm_max_context_tokens": 100_000,
        "pin_recent_turns": 3,
    }}});
    let config = Config::from_document(document).expect("the kept keys load");
    let defaults = &config.agents.defaults;
    let marks = defaults.context_marks().unwrap();
    assert_eq!((marks.high(), marks.low()), (120_000, 40_000));
    assert_eq!(defaults.max_context_tokens, 200_000);
    assert_eq!(defaults.swarm_max_context_tokens, 100_000);
    assert_eq!(defaults.pin_recent_turns, 3);
    let env = HashMap::from([
        (
            "QUECTO_CONTEXT_HIGH_TOKENS".to_string(),
            "100000".to_string(),
        ),
        ("QUECTO_CONTEXT_LOW_TOKENS".to_string(), "30000".to_string()),
        (
            "QUECTO_MAX_CONTEXT_TOKENS".to_string(),
            "150000".to_string(),
        ),
    ]);
    let config = config
        .with_env_overrides(&env)
        .expect("the kept overrides apply");
    assert_eq!(config.agents.defaults.max_context_tokens, 150_000);
    let marks = config.agents.defaults.context_marks().unwrap();
    assert_eq!((marks.high(), marks.low()), (100_000, 30_000));
}
