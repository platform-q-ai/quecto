//! PR #1048 follow-up: the CLI build path (`build_agent_from_config`) must
//! thread the context-management knobs (#1044/#1045/#1046) into the agent so
//! non-default user config is never silently ignored at a construction site.
//! Pattern mirrors `agent_935_clamp_tests.rs`. Kept in its own file for the
//! 750-line source gate.

use super::*;

fn flags_for_wiring_test() -> AgentFlags {
    AgentFlags {
        session_name: None,
        no_session: false,
        message: Some("hi".into()),
        system_prompt: None,
        model_override: Some("fireworks/small-window".into()),
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
        parent_id: None,
    }
}

#[test]
fn build_agent_from_config_threads_context_knobs_into_the_loop() {
    // Non-default context knobs in config.json plus a model with a declared
    // context window smaller than the configured budget: all of them must
    // reach the built loop. Dropping the context-knob wiring (or any future
    // construction site forgetting it) makes this FAIL.
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"providers":{"fireworks":{"api_key":"k"}},"agents":{"defaults":{"max_context_tokens":200000,"pin_recent_turns":5,"context_collapse_after_messages":7}}}"#,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"fireworks":{"api":"openai-completions","baseUrl":"https://e.example/v1","apiKey":"k","models":[{"id":"small-window","contextWindow":100000}]}}}"#,
    )
    .unwrap();
    let flags = flags_for_wiring_test();
    let mut stderr = String::new();
    let cfg = tmp.path().join("config.json");
    let result = build_agent_from_config(tmp.path(), &cfg, false, &flags, &mut stderr, None)
        .unwrap_or_else(|| panic!("build failed; stderr: {stderr}"));
    assert_eq!(
        result.agent.effective_max_context_tokens(),
        100_000,
        "the model's declared window must bound the effective budget (#1044)"
    );
    let (pin, collapse_after_messages) = result.agent.context_knob_snapshot();
    assert_eq!(
        pin, 5,
        "a non-default pin_recent_turns in config must reach the loop (#1045)"
    );
    assert_eq!(
        collapse_after_messages, 7,
        "a non-default context_collapse_after_messages must reach the loop (#1046)"
    );
}
