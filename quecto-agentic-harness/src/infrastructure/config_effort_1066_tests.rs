// Issue #1066: env-var configuration surface for the reasoning-effort
// vocabulary. Split from config.rs's inline tests for the 750-line limit.

use super::Config;
use std::collections::HashMap;

/// Issue #1066: the full OpenAI-documented effort scale is configurable
/// via QUECTO_AGENTS_DEFAULTS_EFFORT — "none" and "xhigh" must be applied,
/// not silently ignored as unrecognised.
#[test]
fn test_env_effort_accepts_openai_documented_scale_1066() {
    for level in ["none", "xhigh"] {
        let mut env = HashMap::new();
        env.insert(
            "QUECTO_AGENTS_DEFAULTS_EFFORT".to_string(),
            level.to_string(),
        );
        let cfg = Config::load_with_env("/nonexistent/config.json", &env).unwrap();
        assert_eq!(
            cfg.agents.defaults.effort.as_deref(),
            Some(level),
            "env effort '{level}' must be applied (#1066)"
        );
    }
}

/// Issue #1066 (review): pin the env-var surface's behaviour for unknown
/// effort values. Env overrides in this config deliberately degrade
/// gracefully (an invalid QUECTO_AGENTS_DEFAULTS_MAX_TOKENS is likewise
/// ignored) so a stale exported variable cannot brick every CLI
/// invocation; strict rejection with a named-values error is the CLI
/// flag surface's job (`--effort`, see flag_parse.rs).
#[test]
fn test_env_effort_ignores_unknown_value_1066() {
    let mut env = HashMap::new();
    env.insert(
        "QUECTO_AGENTS_DEFAULTS_EFFORT".to_string(),
        "turbo".to_string(),
    );
    let cfg = Config::load_with_env("/nonexistent/config.json", &env).unwrap();
    assert_eq!(
        cfg.agents.defaults.effort, None,
        "unknown env effort must be ignored, not applied (#1066)"
    );
}
