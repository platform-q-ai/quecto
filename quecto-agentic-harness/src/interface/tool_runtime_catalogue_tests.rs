use crate::domain::tool_descriptor::{ToolAvailability, ToolHealth, ToolRestrictionReason};
use crate::interface::tool_runtime::{
    ToolEntrypoint, ToolRuntimeBuildArgs, ToolRuntimeWorkflowPolicy, build_tool_runtime,
};

fn build_runtime_with_flags(
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
        entrypoint: ToolEntrypoint::Repl,
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
        disabled_tools,
        workflow: ToolRuntimeWorkflowPolicy::disabled(tmp.path(), Some(tmp.path())),
        stderr: &mut stderr,
    })
    .expect("runtime should build")
}

#[test]
fn repl_catalogue_marks_entrypoint_default_restrictions() {
    let built = build_runtime_with_flags(false, &[]);

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
    let built = build_runtime_with_flags(true, &["write".to_string()]);

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
