// Env-var configuration surface tests, incl. the #1066 reasoning-effort
// vocabulary. Split from config.rs's inline tests for the 750-line limit.

use super::Config;
use std::collections::HashMap;

/// Exercises every env-override branch on top of the default config
/// (zero-config path: no file present). Moved from config.rs's inline
/// tests for the 750-line limit.
#[test]
fn test_env_overrides_cover_all_keys_on_default() {
    let mut env = HashMap::new();
    env.insert(
        "QUECTO_AGENTS_DEFAULTS_WORKSPACE".to_string(),
        "/ws".to_string(),
    );
    env.insert(
        "QUECTO_AGENTS_DEFAULTS_MAX_SESSION_MESSAGES".to_string(),
        "42".to_string(),
    );
    env.insert("QUECTO_MAX_CONTEXT_TOKENS".to_string(), "12345".to_string());
    env.insert("ANTHROPIC_API_KEY".to_string(), "ant-key".to_string());
    env.insert(
        "QUECTO_AGENTS_DEFAULTS_EFFORT".to_string(),
        "high".to_string(),
    );
    env.insert(
        "QUECTO_TOOLS_WEB_BRAVE_API_KEY".to_string(),
        "brave-key".to_string(),
    );
    let cfg = Config::load_with_env("/nonexistent/config.json", &env).unwrap();
    assert_eq!(cfg.agents.defaults.workspace, "/ws");
    assert_eq!(cfg.agents.defaults.max_session_messages, 42);
    assert_eq!(cfg.agents.defaults.max_context_tokens, 12345);
    assert_eq!(cfg.providers.anthropic.api_key, "ant-key");
    assert_eq!(cfg.agents.defaults.effort.as_deref(), Some("high"));
    assert_eq!(cfg.tools.web.brave.api_key, "brave-key");
}

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

/// Issue #1066: an unknown effort value on the env-var configuration
/// surface must be rejected at configuration time with a clear error
/// naming every valid value — not silently ignored.
#[test]
fn test_env_effort_rejects_unknown_value_naming_valid_values_1066() {
    let mut env = HashMap::new();
    env.insert(
        "QUECTO_AGENTS_DEFAULTS_EFFORT".to_string(),
        "turbo".to_string(),
    );
    let err = Config::load_with_env("/nonexistent/config.json", &env)
        .expect_err("unknown env effort must be rejected (#1066)")
        .to_string();
    assert!(
        err.contains("invalid effort level 'turbo'"),
        "error must name the offending value (#1066): {err}"
    );
    for level in ["none", "low", "medium", "high", "xhigh", "max"] {
        assert!(
            err.contains(level),
            "error must name valid value '{level}' (#1066): {err}"
        );
    }
}
