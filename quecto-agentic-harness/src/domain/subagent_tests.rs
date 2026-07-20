use super::*;

#[test]
fn test_validate_agent_id_allowed() {
    let allowlist = vec!["news-bot".to_string(), "weather-bot".to_string()];
    assert!(validate_agent_id("news-bot", &allowlist).is_ok());
}

#[test]
fn test_validate_agent_id_rejected() {
    let allowlist = vec!["news-bot".to_string()];
    let result = validate_agent_id("evil-bot", &allowlist);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not allowed"));
}

#[test]
fn test_subagent_config_new_fields_default() {
    let cfg = SubagentConfig {
        task: None,
        agent_id: None,
        restrict_to_workspace: true,
        system: None,
        config_path: None,
        workflow: false,
        workflow_guards: false,
        workflow_spec: None,
        model: None,
        effort: None,
        disable_tools: Vec::new(),
        read_only: false,
    };
    assert!(cfg.config_path.is_none());
    assert!(!cfg.workflow);
    assert!(!cfg.workflow_guards);
}

#[test]
fn test_subagent_config_with_config_path() {
    let cfg = SubagentConfig {
        task: None,
        agent_id: None,
        restrict_to_workspace: true,
        system: None,
        config_path: Some(PathBuf::from("/custom/config.json")),
        workflow: false,
        workflow_guards: false,
        workflow_spec: None,
        model: None,
        effort: None,
        disable_tools: Vec::new(),
        read_only: false,
    };
    assert_eq!(cfg.config_path, Some(PathBuf::from("/custom/config.json")));
}

#[test]
fn test_subagent_config_with_workflow() {
    let cfg = SubagentConfig {
        task: None,
        agent_id: None,
        restrict_to_workspace: true,
        system: None,
        config_path: None,
        workflow: true,
        workflow_guards: true,
        workflow_spec: None,
        model: None,
        effort: None,
        disable_tools: Vec::new(),
        read_only: false,
    };
    assert!(cfg.workflow);
    assert!(cfg.workflow_guards);
}

// --- parse_model_arg (#881) ---

#[test]
fn test_parse_model_arg_full_string() {
    let arg = parse_model_arg(Some("openai/gpt-5.5"), None, None)
        .unwrap()
        .unwrap();
    assert_eq!(arg, ModelArg::Full("openai/gpt-5.5".to_string()));
    assert_eq!(arg.to_model_string(), "openai/gpt-5.5");
}

#[test]
fn test_parse_model_arg_provider_model_id_pair() {
    let arg = parse_model_arg(None, Some("openai"), Some("gpt-5.5"))
        .unwrap()
        .unwrap();
    assert_eq!(arg.to_model_string(), "openai/gpt-5.5");
}

#[test]
fn test_parse_model_arg_none_is_ok_none() {
    assert_eq!(parse_model_arg(None, None, None).unwrap(), None);
    // Empty strings are treated as absent.
    assert_eq!(parse_model_arg(Some(""), Some(""), Some("")).unwrap(), None);
}

#[test]
fn test_parse_model_arg_provider_without_model_id_errors() {
    let err = parse_model_arg(None, Some("openai"), None).unwrap_err();
    assert!(err.contains("model_id"), "got: {err}");
}

#[test]
fn test_parse_model_arg_model_id_without_provider_errors() {
    let err = parse_model_arg(None, None, Some("gpt-5.5")).unwrap_err();
    assert!(err.contains("provider"), "got: {err}");
}

#[test]
fn test_parse_model_arg_full_takes_precedence_over_pair() {
    let arg = parse_model_arg(Some("a/b"), Some("openai"), Some("gpt-5.5"))
        .unwrap()
        .unwrap();
    assert_eq!(arg, ModelArg::Full("a/b".to_string()));
}
