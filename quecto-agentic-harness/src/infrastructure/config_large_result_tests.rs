//! #2348: the size-aware collapse's dials.

use super::Config;
use crate::application::context_pruning::large_results::LargeResultCollapse;

#[test]
fn the_size_aware_collapse_defaults_to_2k_tokens_seen_for_3_turns() {
    let defaults = Config::default().agents.defaults;
    assert_eq!(defaults.context_collapse_large_result_tokens, 2_000);
    assert_eq!(defaults.context_collapse_large_result_after_turns, 3);
    assert_eq!(
        defaults.large_result_collapse(),
        LargeResultCollapse {
            over_tokens: 2_000,
            after_turns: 3,
        }
    );
}

#[test]
fn a_config_file_sets_the_size_aware_collapse() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(
        &path,
        r#"{"agents":{"defaults":{"context_collapse_large_result_tokens":4000,
            "context_collapse_large_result_after_turns":5}}}"#,
    )
    .unwrap();
    let config = Config::load(path.to_str().unwrap()).unwrap();
    assert_eq!(
        config.agents.defaults.large_result_collapse(),
        LargeResultCollapse {
            over_tokens: 4_000,
            after_turns: 5,
        }
    );
}

fn env(pairs: &[(&str, &str)]) -> std::collections::HashMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

const TOKENS: &str = "QUECTO_CONTEXT_COLLAPSE_LARGE_RESULT_TOKENS";
const TURNS: &str = "QUECTO_CONTEXT_COLLAPSE_LARGE_RESULT_AFTER_TURNS";

/// Like the other context dials, an environment override; a value that is
/// no count is ignored as the other overrides ignore it.
#[test]
fn environment_overrides_set_the_size_aware_collapse() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing.json");
    let path = path.to_str().unwrap();
    let config = Config::load_with_env(path, &env(&[(TOKENS, "8000"), (TURNS, "2")])).unwrap();
    assert_eq!(
        config.agents.defaults.large_result_collapse(),
        LargeResultCollapse {
            over_tokens: 8_000,
            after_turns: 2,
        }
    );
    let config = Config::load_with_env(path, &env(&[(TOKENS, "many"), (TURNS, "-1")])).unwrap();
    assert_eq!(
        config.agents.defaults.context_collapse_large_result_tokens,
        2_000
    );
    assert_eq!(
        config
            .agents
            .defaults
            .context_collapse_large_result_after_turns,
        3
    );
}

/// #2213: a result the model has not seen is never stubbed, so 0 turns is
/// refused at load, from the file or the environment, naming the key.
#[test]
fn zero_turns_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(
        &path,
        r#"{"agents":{"defaults":{"context_collapse_large_result_after_turns":0}}}"#,
    )
    .unwrap();
    let error = Config::load(path.to_str().unwrap()).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("context_collapse_large_result_after_turns"),
        "{error}"
    );
    let missing = dir.path().join("missing.json");
    let error =
        Config::load_with_env(missing.to_str().unwrap(), &env(&[(TURNS, "0")])).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("context_collapse_large_result_after_turns"),
        "{error}"
    );
}

/// The rule is switched off with a size no result reaches.
#[test]
fn the_largest_size_switches_the_rule_off() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(
        &path,
        format!(
            r#"{{"agents":{{"defaults":{{"context_collapse_large_result_tokens":{}}}}}}}"#,
            usize::MAX
        ),
    )
    .unwrap();
    let config = Config::load(path.to_str().unwrap()).unwrap();
    assert_eq!(
        config.agents.defaults.large_result_collapse().over_tokens,
        LargeResultCollapse::DISABLED.over_tokens
    );
}
