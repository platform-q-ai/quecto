use crate::domain::tool_descriptor::{
    ProfileAvailabilityScope, ToolAvailability, ToolHealth, ToolRestrictionReason,
};
use crate::interface::tool_runtime::{
    ToolEntrypoint, ToolRuntimeBuildArgs, ToolRuntimeProfileContext, ToolRuntimeWorkflowPolicy,
    build_tool_runtime,
};

fn build_runtime_with_flags(
    profile_context: ToolRuntimeProfileContext,
    spawned: bool,
    disabled_tools: &[String],
) -> crate::interface::tool_runtime::ToolRuntimeBuild {
    build_runtime_with_entrypoint(
        ToolEntrypoint::UdsAgent,
        profile_context,
        spawned,
        disabled_tools,
    )
}

fn build_runtime_with_entrypoint(
    entrypoint: ToolEntrypoint,
    profile_context: ToolRuntimeProfileContext,
    spawned: bool,
    disabled_tools: &[String],
) -> crate::interface::tool_runtime::ToolRuntimeBuild {
    let config = crate::infrastructure::config::Config::default();
    build_runtime_with_config(entrypoint, profile_context, spawned, disabled_tools, config)
}

fn build_runtime_with_config(
    entrypoint: ToolEntrypoint,
    profile_context: ToolRuntimeProfileContext,
    spawned: bool,
    disabled_tools: &[String],
    config: crate::infrastructure::config::Config,
) -> crate::interface::tool_runtime::ToolRuntimeBuild {
    let tmp = tempfile::tempdir().expect("tempdir");
    let client = reqwest::Client::new();
    let workspace = tmp.path().to_path_buf();
    let sandbox = crate::infrastructure::security::sandbox::Sandbox::new(Some(workspace.clone()));
    let exec_options = crate::infrastructure::tools::bash::ExecOptions::default();
    let mut stderr = String::new();

    build_tool_runtime(ToolRuntimeBuildArgs {
        swarm_context: None,
        swarm_participation: crate::infrastructure::tools::swarm_bridge::Participation::shared(),
        entrypoint,
        profile_context,
        base_dir: tmp.path(),
        config: &config,
        http_client: &client,
        web_fetch_tool: None,
        workspace,
        sandbox,
        exec_options,
        session_key: "catalogue-test".to_string(),
        recall: crate::composition::sessions::build_retention_handles(tmp.path()).recall,
        spawned,
        parent_session_name: None,
        parent_config_path: None,
        effort_control: None,
        container_configs: None,
        kill_tool: None,
        disabled_tools,
        inherited_tool_policy: None,
        workflow: ToolRuntimeWorkflowPolicy::disabled(tmp.path(), Some(tmp.path())),
        stderr: &mut stderr,
        environment_registry: None,
    })
    .expect("runtime should build")
}

#[test]
fn spawned_runtime_catalogue_marks_disable_tool_as_spawn_restriction() {
    let built = build_runtime_with_flags(
        ToolRuntimeProfileContext::Child,
        true,
        &["write".to_string()],
    );

    let write = built
        .catalogue_entries
        .iter()
        .find(|entry| entry.name == "write")
        .expect("write should remain registered for catalogue state");
    assert_eq!(
        write.explicit_restriction,
        Some(ToolRestrictionReason::Spawn)
    );
    assert_eq!(write.session_enabled, Some(false));
    assert_eq!(write.runtime_availability, ToolAvailability::Disabled);
    assert!(!write.effective_enabled);
}

#[test]
fn fresh_parent_runtime_catalogue_leaves_unrestricted_tools_available_to_parent_and_child() {
    let built = build_runtime_with_flags(ToolRuntimeProfileContext::Parent, false, &[]);

    for name in [
        "bash", "docs", "edit", "find", "grep", "ls", "read", "recall", "write",
    ] {
        let entry = built
            .catalogue_entries
            .iter()
            .find(|entry| entry.name == name)
            .unwrap_or_else(|| panic!("{name} should be registered in fresh parent runtime"));
        assert_eq!(
            entry.profile_scope, None,
            "fresh/default parent install must not serialize parent-only profile policy for {name}"
        );
        assert_eq!(
            entry.effective_scope,
            ProfileAvailabilityScope::Both,
            "fresh/default parent install should show {name} as [PC] in the TUI"
        );
        assert!(entry.effective_parent_enabled, "{name} parent enabled");
        assert!(entry.effective_child_enabled, "{name} child enabled");
    }
}

#[test]
fn rust_ast_graph_is_registered_but_disabled_by_default() {
    let built = build_runtime_with_flags(ToolRuntimeProfileContext::Parent, false, &[]);

    let entry = built
        .catalogue_entries
        .iter()
        .find(|entry| entry.name == "rust_ast_graph")
        .expect("rust_ast_graph should remain registered for catalogue state");
    assert!(!entry.default_enabled);
    assert_eq!(
        entry.explicit_restriction,
        Some(ToolRestrictionReason::EntrypointDefault)
    );
    assert_eq!(entry.runtime_availability, ToolAvailability::Disabled);
    assert!(!entry.effective_enabled);
    assert_eq!(entry.health, ToolHealth::Disabled);
    assert!(
        !built
            .registry
            .definitions()
            .iter()
            .any(|definition| definition.name == "rust_ast_graph")
    );
}

#[test]
fn rust_ast_graph_config_enables_tool() {
    let mut config = crate::infrastructure::config::Config::default();
    config.tools.rust_ast_graph.enabled = true;

    let built = build_runtime_with_config(
        ToolEntrypoint::CliAgent,
        ToolRuntimeProfileContext::Parent,
        false,
        &[],
        config,
    );

    let entry = built
        .catalogue_entries
        .iter()
        .find(|entry| entry.name == "rust_ast_graph")
        .expect("rust_ast_graph should be registered");
    assert!(entry.default_enabled);
    assert_eq!(entry.explicit_restriction, None);
    assert!(entry.effective_enabled);
    assert!(
        built
            .registry
            .definitions()
            .iter()
            .any(|definition| definition.name == "rust_ast_graph")
    );
}

#[test]
fn fresh_child_runtime_catalogue_leaves_agent_control_tools_available_to_parent_and_child() {
    let built = build_runtime_with_entrypoint(
        ToolEntrypoint::UdsAgent,
        ToolRuntimeProfileContext::Child,
        true,
        &[],
    );

    for name in ["spawn", "agent_cmd"] {
        let entry = built
            .catalogue_entries
            .iter()
            .find(|entry| entry.name == name)
            .unwrap_or_else(|| panic!("{name} should be registered in fresh child runtime"));
        assert_eq!(
            entry.profile_scope, None,
            "fresh/default child install must not serialize parent-only profile policy for {name}"
        );
        assert_eq!(
            entry.effective_scope,
            ProfileAvailabilityScope::Both,
            "fresh/default child install should show {name} as [PC] in the TUI"
        );
        assert!(entry.effective_parent_enabled, "{name} parent enabled");
        assert!(entry.effective_child_enabled, "{name} child enabled");
        assert!(entry.default_enabled, "{name} default enabled");
        assert!(entry.effective_enabled, "{name} effective enabled");
    }
}

#[test]
fn swarm_runtime_omits_workflow_engine_tool_and_guards() {
    let root = tempfile::tempdir().unwrap();
    let mut registry = crate::infrastructure::tools::registry::ToolRegistryImpl::new();
    let config = crate::infrastructure::config::Config::default();
    let mut workflow = ToolRuntimeWorkflowPolicy::disabled(root.path(), None);
    workflow.workflow_disabled = false; // Normal UDS default makes workflow available.
    let state = super::build_workflow_runtime(
        &mut registry,
        ToolEntrypoint::UdsAgent,
        &config,
        workflow,
        &mut String::new(),
        crate::infrastructure::tools::swarm_bridge::Participation::Fixed(true),
    )
    .unwrap();
    assert!(state.is_none());
    assert_eq!(registry.guard_count(), 0);
    assert!(registry.get("workflow").is_none());
    assert!(!registry.definitions().iter().any(|t| t.name == "workflow"));
}

#[test]
fn swarm_runtime_rejects_guards_and_bound_specs_before_loading_files() {
    let root = tempfile::tempdir().unwrap();
    let spec = root.path().join("unread-spec.json");
    std::fs::write(&spec, "not loaded").unwrap();
    for bound in [false, true] {
        let mut registry = crate::infrastructure::tools::registry::ToolRegistryImpl::new();
        let config = crate::infrastructure::config::Config::default();
        let mut workflow = ToolRuntimeWorkflowPolicy::disabled(root.path(), None);
        workflow.workflow_guards = !bound;
        workflow.workflow_spec_path = bound.then_some(spec.as_path());
        let error = super::build_workflow_runtime(
            &mut registry,
            ToolEntrypoint::UdsAgent,
            &config,
            workflow,
            &mut String::new(),
            crate::infrastructure::tools::swarm_bridge::Participation::Fixed(true),
        )
        .unwrap_err();
        assert!(error.contains("workflow is unavailable for swarm agents"));
        assert!(
            spec.exists(),
            "rejection must precede bound-spec consumption"
        );
    }
}
