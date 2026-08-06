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
        ToolEntrypoint::Repl,
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
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = crate::infrastructure::config::Config::default();
    let client = reqwest::Client::new();
    let workspace = tmp.path().to_path_buf();
    let sandbox =
        crate::infrastructure::security::sandbox::Sandbox::new(Some(workspace.clone()), true);
    let exec_options = crate::infrastructure::tools::bash::ExecOptions::default();
    let mut stderr = String::new();

    build_tool_runtime(ToolRuntimeBuildArgs {
        entrypoint,
        profile_context,
        base_dir: tmp.path(),
        config: &config,
        http_client: &client,
        workspace,
        sandbox,
        exec_options,
        session_key: "catalogue-test".to_string(),
        spawned,
        restrict_to_workspace: true,
        parent_session_name: None,
        parent_config_path: None,
        disabled_tools,
        inherited_tool_policy: None,
        workflow: ToolRuntimeWorkflowPolicy::disabled(tmp.path(), Some(tmp.path())),
        stderr: &mut stderr,
    })
    .expect("runtime should build")
}

#[test]
fn repl_catalogue_marks_entrypoint_default_restrictions() {
    let built = build_runtime_with_flags(ToolRuntimeProfileContext::Parent, false, &[]);

    let spawn = built
        .catalogue_entries
        .iter()
        .find(|entry| entry.name == "spawn")
        .expect("spawn should be registered but disabled by REPL defaults");
    assert!(!spawn.default_enabled);
    assert_eq!(
        spawn.explicit_restriction,
        Some(ToolRestrictionReason::EntrypointDefault)
    );
    assert_eq!(spawn.runtime_availability, ToolAvailability::Disabled);
    assert!(!spawn.effective_enabled);
    assert_eq!(spawn.health, ToolHealth::Disabled);
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
