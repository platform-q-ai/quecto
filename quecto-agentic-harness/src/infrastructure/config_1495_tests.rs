use super::*;

#[test]
fn test_deserialize_deprecated_restrict_to_workspace_ignored() {
    let config: Config = serde_json::from_str(
        r#"{"agents":{"defaults":{"restrict_to_workspace":true,"command_allowlist":["echo"]}}}"#,
    )
    .unwrap();
    assert_eq!(
        config.agents.defaults._deprecated_command_allowlist,
        Some(vec!["echo".to_string()])
    );
}

// #1620: command_allowlist is deprecated the same way.
#[test]
fn test_agent_defaults_has_no_deprecated_command_allowlist() {
    let defaults = AgentDefaults::default();
    assert_eq!(defaults._deprecated_command_allowlist, None);
}

#[test]
fn test_deprecated_command_allowlist_round_trips() {
    let json = r#"{
            "agents": {
                "defaults": {
                    "command_allowlist": ["echo", "ls", "cat"]
                }
            }
        }"#;
    let config: Config = serde_json::from_str(json).unwrap();
    assert_eq!(
        config.agents.defaults._deprecated_command_allowlist,
        Some(vec![
            "echo".to_string(),
            "ls".to_string(),
            "cat".to_string()
        ])
    );
    let out = serde_json::to_string(&config).unwrap();
    assert!(
        out.contains(r#""command_allowlist":["echo","ls","cat"]"#),
        "{out}"
    );
    let default_out = serde_json::to_string(&Config::default()).unwrap();
    assert!(!default_out.contains("command_allowlist"));
}
