use super::*;
use crate::interface::cli::agent::flag_parse::AgentFlags;

fn flags() -> AgentFlags {
    AgentFlags {
        session_name: Some("dev".to_string()),
        no_session: false,
        message: None,
        system_prompt: None,
        model_override: None,
        max_iterations: None,
        max_time: None,
        uds_mode: false,
        no_sandbox: false,
        socket_path: None,
        persist: false,
        disabled_tools: Vec::new(),
        effort: None,
        workflow: false,
        workflow_guards: false,
        workflow_disabled: true,
        workflow_spec_path: None,
        parent_id: None,
    }
}

#[test]
fn resolve_agent_model_prefers_explicit_override() {
    assert_eq!(
        resolve_agent_model(Some("openai-api/gpt-5.6-sol"), "anthropic-api/claude"),
        "openai-api/gpt-5.6-sol"
    );
}

#[test]
fn resolve_agent_model_falls_back_to_config_default() {
    assert_eq!(
        resolve_agent_model(None, "anthropic-api/claude-sonnet-4.5"),
        "anthropic-api/claude-sonnet-4.5"
    );
}

#[test]
fn build_tool_registry_warns_when_sandbox_disabled_and_uses_empty_session_for_no_session() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = Config::default();
    let http = reqwest::Client::new();
    let mut flags = flags();
    flags.no_sandbox = true;
    flags.no_session = true;
    let mut stderr = String::new();

    let built = build_tool_registry(ToolRegistryArgs {
        base_dir: tmp.path(),
        config: &config,
        http_client: &http,
        flags: &flags,
        stderr: &mut stderr,
        broadcast_tx: None,
        cwd: tmp.path(),
        home_dir: Some(tmp.path()),
    })
    .unwrap();

    assert!(
        stderr.contains("--no-sandbox is active"),
        "stderr: {stderr}"
    );
    assert_eq!(built.session_key, "");
    assert_eq!(built.model, config.agents.defaults.model);
    assert!(built.notification_rx.is_some());
    assert!(built.subagent_registry.is_some());
}

#[test]
fn build_tool_registry_uses_cli_session_name_and_model_override() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = Config::default();
    let http = reqwest::Client::new();
    let mut flags = flags();
    flags.session_name = Some("named".to_string());
    flags.model_override = Some("openai-api/gpt-5.6-sol".to_string());
    let mut stderr = String::new();

    let built = build_tool_registry(ToolRegistryArgs {
        base_dir: tmp.path(),
        config: &config,
        http_client: &http,
        flags: &flags,
        stderr: &mut stderr,
        broadcast_tx: None,
        cwd: tmp.path(),
        home_dir: Some(tmp.path()),
    })
    .unwrap();

    assert_eq!(built.session_key, Session::build_key("cli", "named"));
    assert_eq!(built.model, "openai-api/gpt-5.6-sol");
    assert!(!built.extension_prompt_snippets.contains("failed"));
}
