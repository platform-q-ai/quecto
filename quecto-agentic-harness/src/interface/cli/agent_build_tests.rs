//! `build_agent_from_config` over hermetic base directories: zero-config
//! defaults, a missing explicit file, invalid JSON, no providers, and the
//! model override (split out of `agent_tests.rs` for the line ceiling).

use super::integration_tests::{args, composed_ctx};
use super::*;

/// The selection a test hands the build path for a file at `path`: explicit
/// (must exist) or the global layer with no overlay candidate.
pub(crate) fn selection_for_test(path: &std::path::Path, must_exist: bool) -> ConfigSelection {
    if must_exist {
        ConfigSelection::Explicit(path.to_path_buf())
    } else {
        ConfigSelection::Layered(crate::application::configuration::dto::ConfigLayers {
            global: path.to_path_buf(),
            overlay: None,
            legacy_local: None,
        })
    }
}
use crate::composition::tool_policy::build_tool_policy_persistence;
use crate::interface::cli::run_with_output;

// ===================================================================
// build_agent_from_config tests
// ===================================================================

#[test]
fn test_build_agent_from_config_no_config_file() {
    let tmp = tempfile::TempDir::new().unwrap();
    let flags = AgentFlags {
        session_name: None,
        no_session: false,
        message: Some("hi".into()),
        system_prompt: None,
        model_override: None,
        max_iterations: None,
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
        spawned: false,
        parent_identity_override: None,
        session_key_override: None,
        cwd_override: None,
        web_fetch_tool_factory: None,
        kill_tool: None,
        retention: Some(crate::composition::sessions::build_retention_handles),
        catalogue: Some(crate::composition::catalogue::build_catalogue_handles),
        provider_runtime: Some(crate::composition::runtime::build_agent_provider),
        tool_policy_persistence: Some(build_tool_policy_persistence),
        admission_context: None,
        parent_control: None,
        configuration: Some(crate::composition::configuration::build_configuration_handles),
        stdin_is_tty: false,
    };
    let mut stderr = String::new();
    let cfg = tmp.path().join("config.json");
    let result = build_agent_from_config(
        tmp.path(),
        &selection_for_test(&cfg, false),
        &flags,
        &mut stderr,
        None,
    );
    assert!(result.is_none());
    assert!(stderr.contains("no LLM providers configured"));
}

#[test]
fn test_build_agent_from_config_explicit_missing_errors() {
    let tmp = tempfile::TempDir::new().unwrap();
    let flags = AgentFlags {
        session_name: None,
        no_session: false,
        message: Some("hi".into()),
        system_prompt: None,
        model_override: None,
        max_iterations: None,
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
        spawned: false,
        parent_identity_override: None,
        session_key_override: None,
        cwd_override: None,
        web_fetch_tool_factory: None,
        kill_tool: None,
        retention: Some(crate::composition::sessions::build_retention_handles),
        catalogue: Some(crate::composition::catalogue::build_catalogue_handles),
        provider_runtime: Some(crate::composition::runtime::build_agent_provider),
        tool_policy_persistence: Some(build_tool_policy_persistence),
        admission_context: None,
        parent_control: None,
        configuration: Some(crate::composition::configuration::build_configuration_handles),
        stdin_is_tty: false,
    };
    let mut stderr = String::new();
    // An explicit --config (config_explicit = true) pointing at a missing file
    // must error "config not found", not silently fall back to defaults.
    let missing = tmp.path().join("nope.json");
    let result = build_agent_from_config(
        tmp.path(),
        &selection_for_test(&missing, true),
        &flags,
        &mut stderr,
        None,
    );
    assert!(result.is_none());
    assert!(stderr.contains("config not found"), "stderr: {stderr}");
}

#[test]
fn test_build_agent_from_config_invalid_json() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(tmp.path().join("config.json"), "not json at all").unwrap();
    let flags = AgentFlags {
        session_name: None,
        no_session: false,
        message: Some("hi".into()),
        system_prompt: None,
        model_override: None,
        max_iterations: None,
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
        spawned: false,
        parent_identity_override: None,
        session_key_override: None,
        cwd_override: None,
        web_fetch_tool_factory: None,
        kill_tool: None,
        retention: Some(crate::composition::sessions::build_retention_handles),
        catalogue: Some(crate::composition::catalogue::build_catalogue_handles),
        provider_runtime: Some(crate::composition::runtime::build_agent_provider),
        tool_policy_persistence: Some(build_tool_policy_persistence),
        admission_context: None,
        parent_control: None,
        configuration: Some(crate::composition::configuration::build_configuration_handles),
        stdin_is_tty: false,
    };
    let mut stderr = String::new();
    let cfg = tmp.path().join("config.json");
    let result = build_agent_from_config(
        tmp.path(),
        &selection_for_test(&cfg, false),
        &flags,
        &mut stderr,
        None,
    );
    assert!(result.is_none());
    assert!(stderr.contains("failed to load config"));
}

#[test]
fn test_build_agent_from_config_no_providers() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"providers":{"openai":{"api_key":""},"anthropic":{"api_key":""}}}"#,
    )
    .unwrap();
    let flags = AgentFlags {
        session_name: None,
        no_session: false,
        message: Some("hi".into()),
        system_prompt: None,
        model_override: None,
        max_iterations: None,
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
        spawned: false,
        parent_identity_override: None,
        session_key_override: None,
        cwd_override: None,
        web_fetch_tool_factory: None,
        kill_tool: None,
        retention: Some(crate::composition::sessions::build_retention_handles),
        catalogue: Some(crate::composition::catalogue::build_catalogue_handles),
        provider_runtime: Some(crate::composition::runtime::build_agent_provider),
        tool_policy_persistence: Some(build_tool_policy_persistence),
        admission_context: None,
        parent_control: None,
        configuration: Some(crate::composition::configuration::build_configuration_handles),
        stdin_is_tty: false,
    };
    let mut stderr = String::new();
    let cfg = tmp.path().join("config.json");
    let result = build_agent_from_config(
        tmp.path(),
        &selection_for_test(&cfg, false),
        &flags,
        &mut stderr,
        None,
    );
    assert!(result.is_none());
    assert!(stderr.contains("no LLM providers"));
}

#[test]
fn test_build_agent_from_config_with_model_override() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#,
    )
    .unwrap();
    let flags = AgentFlags {
        session_name: None,
        no_session: false,
        message: Some("hi".into()),
        system_prompt: None,
        model_override: Some("gpt-custom".into()),
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
        spawned: false,
        parent_identity_override: None,
        session_key_override: None,
        cwd_override: None,
        web_fetch_tool_factory: None,
        kill_tool: None,
        retention: Some(crate::composition::sessions::build_retention_handles),
        catalogue: Some(crate::composition::catalogue::build_catalogue_handles),
        provider_runtime: Some(crate::composition::runtime::build_agent_provider),
        tool_policy_persistence: Some(build_tool_policy_persistence),
        admission_context: None,
        parent_control: None,
        configuration: Some(crate::composition::configuration::build_configuration_handles),
        stdin_is_tty: false,
    };
    let mut stderr = String::new();
    let cfg = tmp.path().join("config.json");
    let result = build_agent_from_config(
        tmp.path(),
        &selection_for_test(&cfg, false),
        &flags,
        &mut stderr,
        None,
    );
    assert!(result.is_some(), "stderr: {}", stderr);
}

// ===================================================================
// Agent with anthropic provider config
// ===================================================================

#[test]
fn test_agent_with_anthropic_provider_reaches_session() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"providers":{"anthropic":{"api_key":"sk-ant-fake-key"}}}"#,
    )
    .unwrap();
    let ctx = composed_ctx(tmp.path());
    let out = run_with_output(args("agent -m test-anthropic"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(!out.stderr.contains("config not found"));
    assert!(!out.stderr.contains("no LLM providers"));
}

#[test]
fn test_agent_with_both_providers_reaches_session() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"providers":{"openai":{"api_key":"sk-openai-fake"},"anthropic":{"api_key":"sk-ant-fake"}}}"#,
    )
    .unwrap();
    let ctx = composed_ctx(tmp.path());
    let out = run_with_output(args("agent -m test-both"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(!out.stderr.contains("config not found"));
    assert!(!out.stderr.contains("no LLM providers"));
}
