//! #2342: a swarm member's pruning ceiling.

use super::Config;

#[test]
fn a_swarm_members_ceiling_defaults_to_48k_under_the_general_budget() {
    let defaults = Config::default().agents.defaults;
    assert_eq!(defaults.swarm_max_context_tokens, 48_000);
    assert!(defaults.swarm_max_context_tokens < defaults.max_context_tokens);
}

#[test]
fn a_config_file_sets_a_swarm_members_ceiling() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(
        &path,
        r#"{"agents":{"defaults":{"swarm_max_context_tokens":64000}}}"#,
    )
    .unwrap();
    let config = Config::load(path.to_str().unwrap()).unwrap();
    assert_eq!(config.agents.defaults.swarm_max_context_tokens, 64_000);
    assert_eq!(config.agents.defaults.max_context_tokens, 200_000);
}

fn env(value: &str) -> std::collections::HashMap<String, String> {
    [(
        "QUECTO_SWARM_MAX_CONTEXT_TOKENS".to_string(),
        value.to_string(),
    )]
    .into()
}

/// #2349 review L3: like `QUECTO_MAX_CONTEXT_TOKENS`, an environment
/// override; a value that is no count is ignored like the other overrides.
#[test]
fn an_environment_override_sets_a_swarm_members_ceiling() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing.json");
    let path = path.to_str().unwrap();
    let config = Config::load_with_env(path, &env("64000")).unwrap();
    assert_eq!(config.agents.defaults.swarm_max_context_tokens, 64_000);
    let config = Config::load_with_env(path, &env("many")).unwrap();
    assert_eq!(config.agents.defaults.swarm_max_context_tokens, 48_000);
}

/// #2349 review L3: 0 would prune a member to nothing on every request, so
/// it is refused at load, from the file or the environment, naming the key.
/// The cap is switched off by setting it at or above `max_context_tokens`.
#[test]
fn a_zero_swarm_ceiling_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(
        &path,
        r#"{"agents":{"defaults":{"swarm_max_context_tokens":0}}}"#,
    )
    .unwrap();
    let error = Config::load(path.to_str().unwrap()).unwrap_err();
    assert!(
        error.to_string().contains("swarm_max_context_tokens"),
        "{error}"
    );
    let missing = dir.path().join("missing.json");
    let error = Config::load_with_env(missing.to_str().unwrap(), &env("0")).unwrap_err();
    assert!(
        error.to_string().contains("swarm_max_context_tokens"),
        "{error}"
    );
}
