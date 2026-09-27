//! #2216: a spawned child explicitly asked for workflow mode refuses to start
//! when its engine was expected but could not be built (a missing or
//! unreadable spec), naming the load error, so its parent's `spawn` fails
//! instead of reporting a child with no workflow. A top-level agent keeps its
//! fail-closed start, and a swarm member, which builds no engine by design,
//! is not refused.
use crate::infrastructure::config::Config;
use crate::interface::tool_runtime::{
    ToolEntrypoint, ToolRuntimeBuild, ToolRuntimeBuildArgs, ToolRuntimeProfileContext,
    ToolRuntimeWorkflowPolicy, build_tool_runtime,
};

struct Launch<'a> {
    spawned: bool,
    workflow_requested: bool,
    spec: Option<&'a std::path::Path>,
    swarm_context: Option<crate::infrastructure::tools::swarm_bridge::SwarmContext>,
}

fn build(root: &std::path::Path, launch: Launch<'_>) -> Result<ToolRuntimeBuild, String> {
    let client = reqwest::Client::new();
    let mut stderr = String::new();
    let config = Config::default();
    build_tool_runtime(ToolRuntimeBuildArgs {
        swarm_context: launch.swarm_context,
        swarm_participation: crate::infrastructure::tools::swarm_bridge::Participation::shared(),
        entrypoint: ToolEntrypoint::UdsAgent,
        profile_context: ToolRuntimeProfileContext::from_spawned(launch.spawned),
        base_dir: root,
        config: &config,
        http_client: &client,
        web_fetch_tool: None,
        workspace: root.to_path_buf(),
        sandbox: crate::infrastructure::security::sandbox::Sandbox::new(Some(root.to_path_buf())),
        exec_options: Default::default(),
        session_key: "workflow-engine".into(),
        recall: crate::composition::sessions::build_retention_handles(root).recall,
        spawned: launch.spawned,
        parent_session_name: None,
        parent_config_path: None,
        effort_control: None,
        container_configs: None,
        kill_tool: None,
        disabled_tools: &[],
        inherited_tool_policy: None,
        workflow: ToolRuntimeWorkflowPolicy {
            workflow_disabled: false,
            workflow_requested: launch.workflow_requested,
            workflow_guards: false,
            workflow_spec_path: launch.spec,
            broadcast_tx: None,
            emitter_agent_id: None,
            emitter_parent_id: None,
            cwd: root,
            home_dir: Some(root),
        },
        stderr: &mut stderr,
        environment_registry: None,
    })
}

const SPEC: &str = r#"{"template":{"id":"rev","label":"Rev","description":"d","steps":[{"key":"a","label":"A","phase":"review"}]}}"#;

#[test]
fn spawned_child_whose_spec_is_missing_refuses_with_the_load_error() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("missing-spec.json");
    let error = build(
        root.path(),
        Launch {
            spawned: true,
            workflow_requested: false,
            spec: Some(&missing),
            swarm_context: None,
        },
    )
    .err()
    .expect("an expected engine that cannot be built refuses the start");
    assert!(
        error.contains("workflow mode was requested")
            && error.contains("failed to load workflow spec")
            && error.contains("missing-spec.json")
            && error.contains("refusing to start"),
        "got: {error}"
    );
}

#[test]
fn spawned_child_whose_spec_is_unreadable_refuses_with_the_load_error() {
    let root = tempfile::tempdir().unwrap();
    let spec = root.path().join("bad-spec.json");
    std::fs::write(&spec, "{not json").unwrap();
    let error = build(
        root.path(),
        Launch {
            spawned: true,
            workflow_requested: true,
            spec: Some(&spec),
            swarm_context: None,
        },
    )
    .err()
    .expect("an unreadable spec refuses the start");
    assert!(
        error.contains("failed to load workflow spec") && error.contains("bad-spec.json"),
        "got: {error}"
    );
}

#[test]
fn spawned_child_with_a_loadable_spec_starts_with_its_engine() {
    let root = tempfile::tempdir().unwrap();
    let spec = root.path().join("spec.json");
    std::fs::write(&spec, SPEC).unwrap();
    let built = build(
        root.path(),
        Launch {
            spawned: true,
            workflow_requested: false,
            spec: Some(&spec),
            swarm_context: None,
        },
    )
    .expect("child starts");
    assert!(built.workflow_state.is_some());
}

#[test]
fn top_level_agent_whose_spec_is_missing_still_starts_without_an_engine() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("missing-spec.json");
    let built = build(
        root.path(),
        Launch {
            spawned: false,
            workflow_requested: false,
            spec: Some(&missing),
            swarm_context: None,
        },
    )
    .expect("a top-level agent keeps its fail-closed start");
    assert!(built.workflow_state.is_none());
}

#[test]
fn spawned_swarm_member_asked_for_workflow_starts_without_an_engine() {
    let (swarm_dir, swarm) = crate::swarm_control_fixture::context();
    let built = build(
        swarm_dir.path(),
        Launch {
            spawned: true,
            workflow_requested: true,
            spec: None,
            swarm_context: Some(swarm),
        },
    )
    .expect("a swarm member builds no engine by design and is not refused");
    assert!(built.workflow_state.is_none());
}
