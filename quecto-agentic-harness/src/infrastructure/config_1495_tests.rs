use super::*;

#[test]
fn test_deserialize_deprecated_restrict_to_workspace_ignored() {
    let config: Config = serde_json::from_str(
        r#"{"agents":{"defaults":{"restrict_to_workspace":true,"command_allowlist":["echo"]}}}"#,
    )
    .unwrap();
    assert_eq!(
        config.agents.defaults.command_allowlist,
        Some(vec!["echo".to_string()])
    );
}
