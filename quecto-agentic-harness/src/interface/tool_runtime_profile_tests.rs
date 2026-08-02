use crate::domain::tool::ToolProfileContext;
use crate::interface::tool_runtime::{
    ToolEntrypoint, ToolRuntimeBuildArgs, ToolRuntimeProfileContext, ToolRuntimeWorkflowPolicy,
    build_tool_runtime,
};

fn runtime(
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
    let mut stderr = String::new();

    build_tool_runtime(ToolRuntimeBuildArgs {
        entrypoint: ToolEntrypoint::CliAgent,
        profile_context,
        base_dir: tmp.path(),
        config: &config,
        http_client: &client,
        workspace,
        sandbox,
        exec_options: crate::infrastructure::tools::bash::ExecOptions::default(),
        session_key: "profile-test".to_string(),
        spawned,
        restrict_to_workspace: true,
        parent_session_name: None,
        disabled_tools,
        inherited_tool_policy: None,
        workflow: ToolRuntimeWorkflowPolicy::disabled(tmp.path(), Some(tmp.path())),
        stderr: &mut stderr,
    })
    .expect("runtime should build")
}

fn names_for(
    built: &crate::interface::tool_runtime::ToolRuntimeBuild,
    context: ToolProfileContext,
) -> std::collections::BTreeSet<String> {
    built
        .registry
        .definitions_for(context)
        .iter()
        .map(|definition| definition.name.to_string())
        .collect()
}

#[test]
fn tool_visibility_is_selected_by_runtime_profile_not_spawned_role_bit() {
    let parent = runtime(ToolRuntimeProfileContext::Parent, false, &[]);
    let parent_names = names_for(&parent, ToolProfileContext::Parent);
    for name in [
        "agent_cmd",
        "bash",
        "docs",
        "edit",
        "find",
        "grep",
        "ls",
        "read",
        "recall",
        "spawn",
        "write",
    ] {
        assert!(
            parent_names.contains(name),
            "parent profile keeps {name} visible; got {parent_names:?}"
        );
    }

    let child_profile_without_spawned_bit = runtime(ToolRuntimeProfileContext::Child, false, &[]);
    let child_names = names_for(
        &child_profile_without_spawned_bit,
        ToolProfileContext::Child,
    );
    assert!(
        child_names.contains("docs"),
        "docs availability is profile-driven, not role-checked"
    );
    assert!(
        child_names.contains("write"),
        "child profile does not imply read-only by itself"
    );
    assert!(
        !child_names.contains("spawn"),
        "conservative child profile hides spawn"
    );
    assert!(
        !child_names.contains("agent_cmd"),
        "conservative child profile hides agent_cmd"
    );

    let spawned_parent_profile = runtime(ToolRuntimeProfileContext::Parent, true, &[]);
    let spawned_parent_names = names_for(&spawned_parent_profile, ToolProfileContext::Parent);
    assert!(
        spawned_parent_names.contains("spawn"),
        "spawned bit alone does not hide tools"
    );
}

#[tokio::test]
async fn spawned_disable_tools_restrictions_are_layered_over_child_profile_policy() {
    let built = runtime(
        ToolRuntimeProfileContext::Child,
        true,
        &["write".to_string(), "edit".to_string()],
    );
    let child_names = names_for(&built, ToolProfileContext::Child);

    assert!(
        !child_names.contains("spawn"),
        "child profile hides parent coordination tools"
    );
    assert!(
        !child_names.contains("write"),
        "spawn read_only/disable_tools restrictions still apply to write"
    );
    assert!(
        !child_names.contains("edit"),
        "spawn read_only/disable_tools restrictions still apply to edit"
    );
    assert!(child_names.contains("docs"));

    let toc = built.registry.execute("docs", "{}").await.unwrap();
    assert!(!toc.is_error);
    assert!(
        !toc.content.contains("quick-start"),
        "child docs content policy omits quick-start; got {toc:?}"
    );
    let quick_start = built
        .registry
        .execute("docs", r#"{"name":"quick-start"}"#)
        .await
        .unwrap();
    assert!(quick_start.is_error);
}

#[tokio::test]
async fn child_profile_blocks_parent_only_tools_even_if_called_directly() {
    let built = runtime(ToolRuntimeProfileContext::Child, true, &[]);

    let spawn_result = built.registry.execute("spawn", "{}").await.unwrap();
    assert!(spawn_result.is_error);
    assert!(spawn_result.content.contains("Child runtime profile"));

    let agent_cmd_result = built.registry.execute("agent_cmd", "{}").await.unwrap();
    assert!(agent_cmd_result.is_error);
    assert!(agent_cmd_result.content.contains("Child runtime profile"));

    let docs_result = built.registry.execute("docs", "{}").await.unwrap();
    assert!(!docs_result.is_error);
}

#[test]
fn inherited_child_policy_snapshot_includes_agent_control_after_child_policy_allows_it() {
    let mut built = runtime(ToolRuntimeProfileContext::Parent, false, &[]);
    let before_policy_change = built
        .registry
        .get("spawn")
        .expect("spawn registered")
        .inherited_child_policy_snapshot_for_spawn()
        .expect("spawn should carry initial inherited policy");
    assert_ne!(
        before_policy_change.get("spawn"),
        Some(&crate::domain::tool_descriptor::ProfileAvailabilityScope::Both),
        "the regression must prove the snapshot is not captured before policy changes"
    );

    built.registry.apply_tool_policy_mutations(
        &[
            crate::domain::tool::ToolPolicyMutation::set_scope(
                "spawn",
                crate::domain::tool_descriptor::ProfileAvailabilityScope::Both,
                "allow child spawning",
            ),
            crate::domain::tool::ToolPolicyMutation::set_scope(
                "agent_cmd",
                crate::domain::tool_descriptor::ProfileAvailabilityScope::Both,
                "allow child coordination",
            ),
        ],
        crate::domain::tool::ToolPolicyApplyMode::ImmediateIfIdle,
    );

    let snapshot = built
        .registry
        .get("spawn")
        .expect("spawn registered")
        .inherited_child_policy_snapshot_for_spawn()
        .expect("spawn should carry inherited policy");

    assert_eq!(
        snapshot.get("spawn"),
        Some(&crate::domain::tool_descriptor::ProfileAvailabilityScope::Both),
        "child-to-grandchild spawn must inherit child-visible spawn policy"
    );
    assert_eq!(
        snapshot.get("agent_cmd"),
        Some(&crate::domain::tool_descriptor::ProfileAvailabilityScope::Both),
        "child-to-grandchild spawn must inherit child-visible agent_cmd policy"
    );
}
