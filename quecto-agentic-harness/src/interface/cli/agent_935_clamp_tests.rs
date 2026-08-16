//! #935: the CLI build path (`build_agent_from_config`) must wire the model's
//! registry output cap into the agent so a model whose real limit is below the
//! configured `max_tokens` is clamped. Kept in its own file so neither this nor
//! `agent_tests.rs` crosses the source line-count gate.

use super::*;

#[test]
fn test_build_agent_from_config_clamps_effective_max_tokens_to_registry_cap() {
    // Configured max_tokens (200000) is above the model's registry maxTokens
    // (65536), so the built agent's effective cap must be the clamped 65536.
    // Removing the `.with_model_max_tokens(model_cap_from_base_dir(...))` wiring
    // in build_agent_from_config makes this FAIL (effective would be 200000).
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"providers":{"fireworks":{"api_key":"k"}},"agents":{"defaults":{"max_tokens":200000}}}"#,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"fireworks":{"api":"openai-completions","baseUrl":"https://e.example/v1","apiKey":"k","models":[{"id":"qwen3p7-plus","maxTokens":65536}]}}}"#,
    )
    .unwrap();
    let flags = AgentFlags {
        session_name: None,
        no_session: false,
        message: Some("hi".into()),
        system_prompt: None,
        model_override: Some("fireworks/qwen3p7-plus".into()),
        max_iterations: Some(5),
        max_time: None,
        uds_mode: false,
        no_sandbox: false,
        socket_path: None,
        persist: false,
        disabled_tools: vec![],
        effort: None,
        workflow: false,
        workflow_guards: false,
        workflow_disabled: false,
        workflow_spec_path: None,
        inherited_tool_policy: None,
        parent_id: None,
        spawned: false,
        parent_identity_override: None,
        session_key_override: None,
    };
    let mut stderr = String::new();
    let cfg = tmp.path().join("config.json");
    let result = build_agent_from_config(tmp.path(), &cfg, false, &flags, &mut stderr, None)
        .expect("agent build should succeed");
    assert_eq!(
        result.agent.effective_max_tokens(),
        65_536,
        "CLI build must clamp the configured 200000 to the model's registry cap"
    );
}
