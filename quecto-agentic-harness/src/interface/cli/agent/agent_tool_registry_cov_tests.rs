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
        inherited_tool_policy: None,
        parent_id: None,
        spawned: false,
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
        config_path: tmp.path(),
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
        config_path: tmp.path(),
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

/// #1319 glue: `flags.spawned` must reach the docs tool installed by
/// `build_tool_registry`. Unit tests on `DocsTool` alone cannot catch a
/// hard-coded `false` at this call site.
#[tokio::test]
async fn build_tool_registry_forwards_spawned_flag_to_docs_tool() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = Config::default();
    let http = reqwest::Client::new();

    // Spawned child path: parent-only quick-start is rejected / omitted.
    let mut spawned_flags = flags();
    spawned_flags.spawned = true;
    let mut stderr = String::new();
    let spawned = build_tool_registry(ToolRegistryArgs {
        base_dir: tmp.path(),
        config_path: tmp.path(),
        config: &config,
        http_client: &http,
        flags: &spawned_flags,
        stderr: &mut stderr,
        broadcast_tx: None,
        cwd: tmp.path(),
        home_dir: Some(tmp.path()),
    })
    .unwrap();

    assert!(
        spawned.registry.get("docs").is_some(),
        "docs tool must remain registered for spawned agents"
    );
    assert!(
        spawned.registry.get("spawn").is_some(),
        "spawn tool must remain available for spawned agents (#1319 non-goal)"
    );

    let toc = spawned.registry.execute("docs", "{}").await.unwrap();
    assert!(!toc.is_error, "spawned TOC must succeed: {}", toc.content);
    assert!(
        !toc.content.contains("quick-start"),
        "spawned registry docs TOC must omit quick-start; got:\n{}",
        toc.content
    );

    let direct = spawned
        .registry
        .execute("docs", r#"{"name":"quick-start"}"#)
        .await
        .unwrap();
    assert!(
        direct.is_error,
        "spawned registry must reject quick-start; got ok:\n{}",
        direct.content
    );
    assert!(
        !direct.content.contains("Parent versus subagent"),
        "spawned registry must not return quick-start body"
    );

    // Top-level path: same glue must keep quick-start available.
    let top_flags = flags(); // spawned: false
    stderr.clear();
    let top = build_tool_registry(ToolRegistryArgs {
        base_dir: tmp.path(),
        config_path: tmp.path(),
        config: &config,
        http_client: &http,
        flags: &top_flags,
        stderr: &mut stderr,
        broadcast_tx: None,
        cwd: tmp.path(),
        home_dir: Some(tmp.path()),
    })
    .unwrap();

    let top_toc = top.registry.execute("docs", "{}").await.unwrap();
    assert!(!top_toc.is_error);
    assert!(
        top_toc.content.contains("quick-start — "),
        "top-level registry docs TOC must list quick-start; got:\n{}",
        top_toc.content
    );
    let top_direct = top
        .registry
        .execute("docs", r#"{"name":"quick-start"}"#)
        .await
        .unwrap();
    assert!(!top_direct.is_error);
    assert!(
        top_direct.content.contains("Parent versus subagent"),
        "top-level registry must still serve quick-start body"
    );
}

/// #1276 Phase 3 characterization: agent-control tools are always present on
/// the CLI composition root with official-native descriptors and are not
/// unloadable via the extension lifecycle path.
#[test]
fn build_tool_registry_registers_agent_control_tools_as_official_native() {
    use crate::domain::tool_descriptor::{ToolAvailability, ToolSource};

    let tmp = tempfile::TempDir::new().unwrap();
    let config = Config::default();
    let http = reqwest::Client::new();
    let flags = flags();
    let mut stderr = String::new();

    let built = build_tool_registry(ToolRegistryArgs {
        base_dir: tmp.path(),
        config_path: tmp.path(),
        config: &config,
        http_client: &http,
        flags: &flags,
        stderr: &mut stderr,
        broadcast_tx: None,
        cwd: tmp.path(),
        home_dir: Some(tmp.path()),
    })
    .unwrap();

    for name in ["recall", "spawn", "agent_cmd"] {
        assert!(
            built.registry.get(name).is_some(),
            "{name} must be registered"
        );
        let descriptor = built
            .registry
            .descriptor(name)
            .unwrap_or_else(|| panic!("missing descriptor for {name}"));
        assert!(
            matches!(descriptor.source, ToolSource::BundledNative),
            "{name} source"
        );
        assert_eq!(descriptor.owner.as_ref(), "quecto:official-tools");
        assert!(matches!(descriptor.availability, ToolAvailability::Enabled));
        assert!(
            !built
                .registry
                .runtime_tool_names()
                .iter()
                .any(|n| n == name),
            "{name} must not be unloadable via unregister_extension"
        );
        assert!(
            built
                .registry
                .definitions()
                .iter()
                .any(|d| d.name.as_ref() == name),
            "{name} must be model-visible"
        );
    }

    assert!(
        built.registry.get("workflow").is_none(),
        "workflow stays off when not uds / disabled"
    );
    assert!(built.notification_rx.is_some());
    assert!(built.subagent_registry.is_some());
    assert!(built.workflow_state.is_none());
}

/// #1276 Phase 3 characterization: UDS + workflow flag wires the workflow tool
/// and keeps the shared engine handle live for guards/binding.
#[test]
fn build_tool_registry_registers_workflow_when_uds_and_enabled() {
    use crate::domain::tool_descriptor::ToolSource;

    let tmp = tempfile::TempDir::new().unwrap();
    let config = Config::default();
    let http = reqwest::Client::new();
    let mut flags = flags();
    flags.uds_mode = true;
    flags.workflow_disabled = false;
    flags.workflow = true;
    flags.workflow_guards = true;
    let mut stderr = String::new();

    let built = build_tool_registry(ToolRegistryArgs {
        base_dir: tmp.path(),
        config_path: tmp.path(),
        config: &config,
        http_client: &http,
        flags: &flags,
        stderr: &mut stderr,
        broadcast_tx: None,
        cwd: tmp.path(),
        home_dir: Some(tmp.path()),
    })
    .unwrap();

    assert!(built.registry.get("workflow").is_some());
    let descriptor = built.registry.descriptor("workflow").unwrap();
    assert!(matches!(descriptor.source, ToolSource::BundledNative));
    assert_eq!(descriptor.owner.as_ref(), "quecto:official-tools");
    assert!(
        !built
            .registry
            .runtime_tool_names()
            .iter()
            .any(|n| n == "workflow")
    );
    assert!(built.workflow_state.is_some());
    assert_eq!(
        built.registry.guard_count(),
        1,
        "workflow_guards must register the guard"
    );
}

/// #1276 Phase 3 characterization: config-gated web tools are bundled native
/// official tools, not runtime-unloadable extension tools.
#[test]
fn build_tool_registry_registers_web_tools_as_bundled_native_official_tools() {
    use crate::domain::tool_descriptor::ToolSource;

    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = Config::default();
    config.tools.web.fetch.enabled = true;
    config.tools.web.brave.enabled = true;
    config.tools.web.brave.api_key = "test-key".into();
    let http = reqwest::Client::new();
    let flags = flags();
    let mut stderr = String::new();

    let built = build_tool_registry(ToolRegistryArgs {
        base_dir: tmp.path(),
        config_path: tmp.path(),
        config: &config,
        http_client: &http,
        flags: &flags,
        stderr: &mut stderr,
        broadcast_tx: None,
        cwd: tmp.path(),
        home_dir: Some(tmp.path()),
    })
    .unwrap();

    for name in ["web_search", "web_fetch"] {
        assert!(built.registry.get(name).is_some(), "{name} registered");
        let descriptor = built.registry.descriptor(name).unwrap();
        assert!(matches!(descriptor.source, ToolSource::BundledNative));
        assert_eq!(descriptor.owner.as_ref(), "quecto:official-tools");
        assert!(
            !built
                .registry
                .runtime_tool_names()
                .iter()
                .any(|n| n == name),
            "{name} must not be runtime-unloadable"
        );
        assert!(
            built
                .registry
                .definitions()
                .iter()
                .any(|d| d.name.as_ref() == name),
            "{name} must remain model-visible"
        );
    }
}
