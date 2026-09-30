//! #2348: the size-aware collapse's dials. Review M1: unset, the rule is
//! on only for a swarm member; set, it applies to every agent.

use super::Config;
use crate::domain::large_result_collapse::LargeResultCollapse;

const SWARM_DEFAULT: LargeResultCollapse = LargeResultCollapse {
    over_tokens: 2_000,
    after_turns: 3,
};

#[test]
fn unset_the_size_aware_collapse_is_off_except_for_a_swarm_member() {
    let defaults = Config::default().agents.defaults;
    assert_eq!(defaults.context_collapse_large_result_tokens, None);
    assert_eq!(defaults.context_collapse_large_result_after_turns, None);
    assert_eq!(
        defaults.large_result_collapse(),
        LargeResultCollapse::DISABLED
    );
    assert_eq!(defaults.swarm_large_result_collapse(), SWARM_DEFAULT);
}

#[test]
fn a_configured_size_applies_to_every_agent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(
        &path,
        r#"{"agents":{"defaults":{"context_collapse_large_result_tokens":4000,
            "context_collapse_large_result_after_turns":5}}}"#,
    )
    .unwrap();
    let defaults = Config::load(path.to_str().unwrap())
        .unwrap()
        .agents
        .defaults;
    let configured = LargeResultCollapse {
        over_tokens: 4_000,
        after_turns: 5,
    };
    assert_eq!(defaults.large_result_collapse(), configured);
    assert_eq!(defaults.swarm_large_result_collapse(), configured);
}

#[test]
fn configured_turns_alone_switch_nothing_on_outside_a_swarm() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(
        &path,
        r#"{"agents":{"defaults":{"context_collapse_large_result_after_turns":5}}}"#,
    )
    .unwrap();
    let defaults = Config::load(path.to_str().unwrap())
        .unwrap()
        .agents
        .defaults;
    assert_eq!(
        defaults.large_result_collapse(),
        LargeResultCollapse::DISABLED
    );
    assert_eq!(
        defaults.swarm_large_result_collapse(),
        LargeResultCollapse {
            after_turns: 5,
            ..SWARM_DEFAULT
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
        None
    );
    assert_eq!(
        config
            .agents
            .defaults
            .context_collapse_large_result_after_turns,
        None
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

/// The rule is switched off, for a swarm member too, with a size no result
/// reaches.
#[test]
fn the_largest_size_switches_the_rule_off_everywhere() {
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
    let defaults = Config::load(path.to_str().unwrap())
        .unwrap()
        .agents
        .defaults;
    assert_eq!(
        defaults.large_result_collapse().over_tokens,
        LargeResultCollapse::DISABLED.over_tokens
    );
    assert_eq!(
        defaults.swarm_large_result_collapse().over_tokens,
        LargeResultCollapse::DISABLED.over_tokens
    );
}
