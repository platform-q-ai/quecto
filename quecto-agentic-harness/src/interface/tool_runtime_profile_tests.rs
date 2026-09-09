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
    let sandbox = crate::infrastructure::security::sandbox::Sandbox::new(Some(workspace.clone()));
    let mut stderr = String::new();

    build_tool_runtime(ToolRuntimeBuildArgs {
        swarm_context: None,
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
        parent_session_name: None,
        parent_config_path: None,
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
        child_names.contains("spawn"),
        "child profile keeps spawn visible by default"
    );
    assert!(
        child_names.contains("agent_cmd"),
        "child profile keeps agent_cmd visible by default"
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
        child_names.contains("spawn"),
        "child profile keeps agent-control tools unless explicitly restricted"
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
        "child runtime omits removed quick-start; got {toc:?}"
    );
    let quick_start = built
        .registry
        .execute("docs", r#"{"name":"quick-start"}"#)
        .await
        .unwrap();
    assert!(quick_start.is_error);
}

#[tokio::test]
async fn child_profile_executes_agent_control_tools_when_not_restricted_by_policy() {
    let built = runtime(ToolRuntimeProfileContext::Child, true, &[]);

    let agent_cmd_result = built.registry.execute("agent_cmd", "{}").await.unwrap();
    assert!(agent_cmd_result.is_error);
    assert!(
        !agent_cmd_result.content.contains("Child runtime profile"),
        "agent_cmd should reach its implementation instead of profile gating"
    );

    let docs_result = built.registry.execute("docs", "{}").await.unwrap();
    assert!(!docs_result.is_error);
}

#[test]
fn inherited_child_policy_snapshot_includes_agent_control_by_default() {
    let built = runtime(ToolRuntimeProfileContext::Parent, false, &[]);

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

#[test]
fn trial_isolated_runtime_build_does_not_enroll_in_full_ambient_swarm() {
    use crate::domain::swarm::{CoordinationPort, ProcessIdentity};
    let tmp = tempfile::tempdir().unwrap();
    let context = crate::infrastructure::tools::swarm_bridge::SwarmContext {
        checkout: tmp.path().into(),
        member: "coordinator".into(),
        lifecycle: std::sync::Arc::new(crate::application::swarm::LifecycleService),
    };
    std::fs::create_dir(tmp.path().join(".quecto")).unwrap();
    let deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 60;
    context
        .create_run(
            &serde_json::json!({"goal":"test isolation","constraints":[],
        "criteria":[{"id":"test","kind":"command","description":"pass"}],
        "member_limit":1,"deadline":deadline}),
            &ProcessIdentity {
                pid: std::process::id(),
                started: crate::infrastructure::tools::swarm_bridge::process_start(
                    std::process::id(),
                )
                .unwrap(),
            },
            None,
        )
        .unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "interface::tool_runtime::profile_tests::tool_visibility_is_selected_by_runtime_profile_not_spawned_role_bit", "--nocapture"])
        .env("QUECTO_SWARM_CHECKOUT", tmp.path())
        .env("QUECTO_SWARM_CONTAINER", "isolated-pid-v1")
        // Emulate inherited container metadata without changing the test host's namespace.
        .env("QUECTO_SWARM_HOST_PID_NS", "pid:[0]")
        .env("QUECTO_SWARM_MEMBER", "ordinary-test-runtime")
        .env_remove("QUECTO_SWARM_RESERVATION")
        .output().unwrap();
    assert!(
        output.status.success(),
        "isolated runtime enrolled in live pool:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        context
            .reserve_member("real-extra-member", "new-reservation")
            .is_err(),
        "actual managed admission must still enforce the full pool"
    );
}

fn workflow_enabled_policy<'a>(
    cwd: &'a std::path::Path,
    home: Option<&'a std::path::Path>,
) -> ToolRuntimeWorkflowPolicy<'a> {
    ToolRuntimeWorkflowPolicy {
        workflow_disabled: false,
        workflow_guards: false,
        workflow_spec_path: None,
        broadcast_tx: None,
        emitter_agent_id: None,
        emitter_parent_id: None,
        cwd,
        home_dir: home,
    }
}

fn runtime_in_container(
    context: crate::infrastructure::tools::swarm_bridge::SwarmContext,
    tmp: &std::path::Path,
) -> Result<crate::interface::tool_runtime::ToolRuntimeBuild, String> {
    let config = crate::infrastructure::config::Config::default();
    let client = reqwest::Client::new();
    let sandbox = crate::infrastructure::security::sandbox::Sandbox::new(Some(tmp.to_path_buf()));
    let mut stderr = String::new();
    build_tool_runtime(ToolRuntimeBuildArgs {
        swarm_context: Some(context),
        // The UDS entrypoint is the one that supports workflows at all.
        entrypoint: ToolEntrypoint::UdsAgent,
        profile_context: ToolRuntimeProfileContext::Child,
        base_dir: tmp,
        config: &config,
        http_client: &client,
        workspace: tmp.to_path_buf(),
        sandbox,
        exec_options: crate::infrastructure::tools::bash::ExecOptions::default(),
        session_key: "container-workflow-test".to_string(),
        spawned: true,
        parent_session_name: None,
        parent_config_path: None,
        disabled_tools: &[],
        inherited_tool_policy: None,
        workflow: workflow_enabled_policy(tmp, Some(tmp)),
        stderr: &mut stderr,
    })
}

/// #1715: composition installs the workflow runtime for an ordinary container
/// (bootstrap placeholder run) and omits it once the run has been created.
#[test]
fn container_runtime_workflow_follows_swarm_participation() {
    use crate::domain::swarm::ProcessIdentity;
    let identity = ProcessIdentity {
        pid: std::process::id(),
        started: crate::infrastructure::tools::swarm_bridge::process_start(std::process::id())
            .unwrap(),
    };
    let ordinary = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(ordinary.path().join(".quecto")).unwrap();
    let context = crate::infrastructure::tools::swarm_bridge::SwarmContext {
        checkout: ordinary.path().to_path_buf(),
        member: "ordinary".into(),
        lifecycle: std::sync::Arc::new(crate::application::swarm::LifecycleService),
    };
    context.join(&identity, None, None).unwrap();
    let built = runtime_in_container(context, ordinary.path()).unwrap();
    assert!(
        built.workflow_state.is_some(),
        "an ordinary container keeps its workflow engine"
    );
    assert!(names_for(&built, ToolProfileContext::Child).contains("workflow"));
    let (swarm_dir, swarm) = crate::swarm_control_fixture::context();
    let built = runtime_in_container(swarm, swarm_dir.path()).unwrap();
    assert!(
        built.workflow_state.is_none(),
        "a created run makes every member a swarm agent"
    );
    assert!(!names_for(&built, ToolProfileContext::Child).contains("workflow"));
}
