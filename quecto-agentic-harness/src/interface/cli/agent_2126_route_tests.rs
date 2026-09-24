//! #2126: the startup model must reach a configured provider, checked on
//! the real build path (production provider composition, not a fake).
use super::build_tests::selection_for_test;
use super::*;

fn flags(model: &str, spawned: bool) -> AgentFlags {
    AgentFlags {
        session_name: None,
        no_session: false,
        message: Some("hi".into()),
        system_prompt: None,
        model_override: Some(model.into()),
        max_iterations: Some(5),
        max_time: None,
        uds_mode: false,
        socket_path: None,
        persist: false,
        disabled_tools: vec![],
        effort: None,
        workflow: false,
        workflow_guards: false,
        workflow_disabled: false,
        swarm_participation: crate::infrastructure::tools::swarm_bridge::Participation::none(),
        workflow_spec_path: None,
        inherited_tool_policy: None,
        parent_id: None,
        spawned,
        parent_identity_override: None,
        session_key_override: None,
        cwd_override: None,
        web_fetch_tool_factory: None,
        kill_tool: None,
        retention: Some(crate::composition::sessions::build_retention_handles),
        catalogue: Some(crate::composition::catalogue::build_catalogue_handles),
        provider_runtime: Some(crate::composition::runtime::build_agent_provider),
        tool_policy_persistence: Some(
            crate::composition::tool_policy::build_tool_policy_persistence,
        ),
        admission_context: None,
        parent_control: None,
        configuration: Some(crate::composition::configuration::build_configuration_handles),
        admission: Some(crate::composition::admission::build_admission_handles),
        container_configs: Some(
            crate::composition::container_configs::build_agent_container_config_handles,
        ),
        stdin_is_tty: false,
        environment_registry: None,
    }
}

fn build(model: &str, spawned: bool) -> (bool, String) {
    let (agent, stderr) = build_agent(model, spawned);
    (agent.is_some(), stderr)
}

fn build_agent(model: &str, spawned: bool) -> (Option<AgentBuildResult>, String) {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"providers":{"fireworks":{"api_key":"k"}}}"#,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"fireworks":{"api":"openai-completions","baseUrl":"https://e.example/v1","apiKey":"k","models":[{"id":"some-model","maxTokens":8192}]}}}"#,
    )
    .unwrap();
    let mut stderr = String::new();
    let cfg = tmp.path().join("config.json");
    let built = build_agent_from_config(
        tmp.path(),
        &selection_for_test(&cfg, false),
        &flags(model, spawned),
        &mut stderr,
        None,
    );
    (built, stderr)
}

#[test]
fn a_spawned_child_on_an_unconfigured_provider_refuses_to_start() {
    let (built, stderr) = build("openai/gpt-5.2", true);
    assert!(!built, "the child must not start");
    assert!(
        stderr.contains("cannot start") && stderr.contains("fireworks"),
        "{stderr}"
    );
}

#[test]
fn the_root_on_an_unconfigured_provider_starts_with_a_warning() {
    let (built, stderr) = build("openai/gpt-5.2", false);
    assert!(built, "the root still starts: {stderr}");
    assert!(
        stderr.contains("warning") && stderr.contains("fireworks"),
        "{stderr}"
    );
}

#[test]
fn a_configured_provider_starts_without_a_warning_either_way() {
    for spawned in [true, false] {
        let (built, stderr) = build("fireworks/some-model", spawned);
        assert!(built, "{stderr}");
        assert!(!stderr.contains("not configured"), "{stderr}");
    }
}

#[test]
fn the_built_agent_refuses_to_route_a_switch_to_an_unconfigured_provider() {
    // The loop that /model and set_model switch must ask the composed router
    // (RetryingProvider over ProviderRouter), not answer "routable" itself.
    use crate::application::catalogue::ports::ModelRuntime;
    use crate::application::providers::ports::RouteCheck;
    let (agent, stderr) = build_agent("fireworks/some-model", false);
    let agent = agent.unwrap_or_else(|| panic!("{stderr}")).agent;
    assert_eq!(agent.route_check("fireworks/other"), RouteCheck::Routable);
    assert!(matches!(
        agent.route_check("openai/gpt-5.2"),
        RouteCheck::UnknownProvider { ref configured, .. } if configured.iter().any(|p| p == "fireworks")
    ));
}
