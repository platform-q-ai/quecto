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
