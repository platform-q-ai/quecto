//! #2216: a child's workflow tool under its parent's inherited tool policy.
//!
//! A one-shot CLI parent never builds the workflow tool, so its inherited
//! snapshot has no workflow entry. That absence is an entrypoint difference,
//! not a denial: a UDS child it launches keeps the workflow tool. A parent
//! whose policy DENIES workflow still caps its children, and a child that was
//! asked for workflow mode but cannot see the tool refuses to start.
use std::collections::BTreeMap;

use crate::domain::tool::ToolProfileContext;
use crate::domain::tool_descriptor::ProfileAvailabilityScope;
use crate::infrastructure::config::{Config, ToolPolicyEntryConfig};
use crate::infrastructure::tools::inherited_tool_policy::{
    InheritedToolPolicySnapshot, workflow_tool_identity,
};
use crate::interface::tool_runtime::{
    ToolEntrypoint, ToolRuntimeBuild, ToolRuntimeBuildArgs, ToolRuntimeProfileContext,
    ToolRuntimeWorkflowPolicy, build_tool_runtime,
};

pub(super) struct Launch<'a> {
    pub(super) entrypoint: ToolEntrypoint,
    pub(super) profile: ToolRuntimeProfileContext,
    pub(super) config: Config,
    pub(super) disabled_tools: &'a [String],
    pub(super) inherited: Option<InheritedToolPolicySnapshot>,
    pub(super) workflow_requested: bool,
    pub(super) workflow_guards: bool,
}

pub(super) fn build(
    root: &std::path::Path,
    launch: Launch<'_>,
) -> Result<ToolRuntimeBuild, String> {
    let client = reqwest::Client::new();
    let mut stderr = String::new();
    let uds = launch.entrypoint == ToolEntrypoint::UdsAgent;
    build_tool_runtime(ToolRuntimeBuildArgs {
        launch_extensions: true,
        swarm_context: None,
        swarm_participation: crate::infrastructure::tools::swarm_bridge::Participation::shared(),
        entrypoint: launch.entrypoint,
        profile_context: launch.profile,
        base_dir: root,
        config: &launch.config,
        http_client: &client,
        web_fetch_tool: None,
        workspace: root.to_path_buf(),
        sandbox: crate::infrastructure::security::sandbox::Sandbox::new(Some(root.to_path_buf())),
        exec_options: Default::default(),
        session_key: "inherited-workflow".into(),
        recall: crate::composition::sessions::build_retention_handles(root).recall,
        spawned: launch.profile == ToolRuntimeProfileContext::Child,
        parent_session_name: None,
        parent_config_path: None,
        effort_control: None,
        container_configs: None,
        kill_tool: None,
        disabled_tools: launch.disabled_tools,
        inherited_tool_policy: launch.inherited,
        workflow: ToolRuntimeWorkflowPolicy {
            workflow_disabled: !uds,
            workflow_requested: launch.workflow_requested,
            workflow_guards: launch.workflow_guards,
            workflow_spec_path: None,
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

/// The snapshot a one-shot CLI parent hands its children.
pub(super) fn cli_parent_snapshot(
    root: &std::path::Path,
    config: Config,
    disabled_tools: &[String],
) -> InheritedToolPolicySnapshot {
    let parent = build(
        root,
        Launch {
            entrypoint: ToolEntrypoint::CliAgent,
            profile: ToolRuntimeProfileContext::Parent,
            config,
            disabled_tools,
            inherited: None,
            workflow_requested: false,
            workflow_guards: false,
        },
    )
    .expect("CLI parent builds");
    assert!(
        parent.registry.get("workflow").is_none(),
        "a one-shot CLI parent never builds the workflow tool"
    );
    let tools = parent
        .registry
        .get("spawn")
        .expect("spawn registered")
        .inherited_child_policy_snapshot_for_spawn()
        .expect("spawn carries the inherited policy");
    InheritedToolPolicySnapshot { version: 1, tools }
}

pub(super) fn uds_child(
    root: &std::path::Path,
    inherited: InheritedToolPolicySnapshot,
    workflow_requested: bool,
    workflow_guards: bool,
) -> Result<ToolRuntimeBuild, String> {
    build(
        root,
        Launch {
            entrypoint: ToolEntrypoint::UdsAgent,
            profile: ToolRuntimeProfileContext::Child,
            config: Config::default(),
            disabled_tools: &[],
            inherited: Some(inherited),
            workflow_requested,
            workflow_guards,
        },
    )
}

pub(super) fn child_sees_workflow(built: &ToolRuntimeBuild) -> bool {
    built
        .registry
        .definitions_for(ToolProfileContext::Child)
        .iter()
        .any(|definition| definition.name.as_ref() == "workflow")
}

fn workflow_policy(scope: ProfileAvailabilityScope) -> Config {
    let mut config = Config::default();
    config.tools.policy.entries.insert(
        workflow_tool_identity().stable_id.into_owned(),
        ToolPolicyEntryConfig { scope },
    );
    config
}

#[test]
fn cli_parent_child_spawned_with_workflow_gets_the_workflow_tool() {
    let root = tempfile::tempdir().unwrap();
    for (requested, guards) in [(true, false), (true, true)] {
        let inherited = cli_parent_snapshot(root.path(), Config::default(), &[]);
        let child = uds_child(root.path(), inherited, requested, guards)
            .expect("the child starts in workflow mode");
        assert!(child.workflow_state.is_some());
        assert!(
            child_sees_workflow(&child),
            "a tool the parent never built is not denied to the child"
        );
    }
}

#[test]
fn cli_parent_plain_child_keeps_the_workflow_tool() {
    let root = tempfile::tempdir().unwrap();
    let inherited = cli_parent_snapshot(root.path(), Config::default(), &[]);
    let child = uds_child(root.path(), inherited, false, false).expect("child builds");
    assert!(child_sees_workflow(&child));
}

#[test]
fn inherited_snapshot_still_closes_tools_that_are_not_entrypoint_only() {
    let root = tempfile::tempdir().unwrap();
    let mut inherited = cli_parent_snapshot(root.path(), Config::default(), &[]);
    inherited
        .tools
        .retain(|id, _| !id.ends_with(":bash") && id != "bash");
    let child = uds_child(root.path(), inherited, true, false).expect("child builds");
    assert!(child_sees_workflow(&child));
    assert!(
        !child
            .registry
            .definitions_for(ToolProfileContext::Child)
            .iter()
            .any(|definition| definition.name.as_ref() == "bash"),
        "only entrypoint-only tools escape the snapshot's closed default"
    );
}

#[test]
fn parent_that_disables_workflow_denies_it_to_children() {
    let root = tempfile::tempdir().unwrap();
    let inherited = cli_parent_snapshot(root.path(), Config::default(), &["workflow".into()]);
    let identity = workflow_tool_identity();
    assert_eq!(
        inherited.tools.get(identity.stable_id.as_ref()),
        Some(&ProfileAvailabilityScope::None),
        "the snapshot records the denial of a tool the parent never built"
    );

    let plain = uds_child(root.path(), inherited.clone(), false, false).expect("child builds");
    assert!(!child_sees_workflow(&plain));

    let error = uds_child(root.path(), inherited, true, false)
        .err()
        .expect("a workflow child that cannot see the tool refuses to start");
    assert!(
        error.contains("workflow mode was requested")
            && error.contains("tool policy denies the workflow tool"),
        "got: {error}"
    );
}

#[test]
fn parent_policy_keeping_workflow_to_itself_denies_it_to_children() {
    let root = tempfile::tempdir().unwrap();
    for scope in [
        ProfileAvailabilityScope::Parent,
        ProfileAvailabilityScope::None,
    ] {
        let inherited = cli_parent_snapshot(root.path(), workflow_policy(scope), &[]);
        assert_eq!(
            inherited
                .tools
                .get(workflow_tool_identity().stable_id.as_ref()),
            Some(&scope)
        );
        assert!(uds_child(root.path(), inherited, true, false).is_err());
    }
    let inherited = cli_parent_snapshot(
        root.path(),
        workflow_policy(ProfileAvailabilityScope::Child),
        &[],
    );
    let child = uds_child(root.path(), inherited, true, false).expect("child builds");
    assert!(child_sees_workflow(&child));
}

#[test]
fn child_whose_own_policy_denies_requested_workflow_refuses_to_start() {
    let root = tempfile::tempdir().unwrap();
    let inherited = cli_parent_snapshot(root.path(), Config::default(), &[]);
    let error = build(
        root.path(),
        Launch {
            entrypoint: ToolEntrypoint::UdsAgent,
            profile: ToolRuntimeProfileContext::Child,
            config: Config::default(),
            disabled_tools: &["workflow".into()],
            inherited: Some(inherited),
            workflow_requested: true,
            workflow_guards: false,
        },
    )
    .err()
    .expect("a child denied its requested workflow tool refuses to start");
    assert!(
        error.contains("workflow mode was requested"),
        "got: {error}"
    );
}

#[test]
fn uds_parent_snapshot_records_its_workflow_scope() {
    let root = tempfile::tempdir().unwrap();
    let parent = build(
        root.path(),
        Launch {
            entrypoint: ToolEntrypoint::UdsAgent,
            profile: ToolRuntimeProfileContext::Parent,
            config: Config::default(),
            disabled_tools: &[],
            inherited: None,
            workflow_requested: false,
            workflow_guards: false,
        },
    )
    .expect("UDS parent builds");
    let tools: BTreeMap<_, _> = parent
        .registry
        .get("spawn")
        .unwrap()
        .inherited_child_policy_snapshot_for_spawn()
        .unwrap();
    assert_eq!(
        tools.get(workflow_tool_identity().stable_id.as_ref()),
        Some(&ProfileAvailabilityScope::Both)
    );
}

#[test]
fn top_level_agent_denied_its_requested_workflow_still_starts() {
    let root = tempfile::tempdir().unwrap();
    let built = build(
        root.path(),
        Launch {
            entrypoint: ToolEntrypoint::UdsAgent,
            profile: ToolRuntimeProfileContext::Parent,
            config: Config::default(),
            disabled_tools: &["workflow".into()],
            inherited: None,
            workflow_requested: true,
            workflow_guards: false,
        },
    )
    .expect("a top-level agent owns its policy; its launch is not refused");
    assert!(
        !built
            .registry
            .definitions_for(ToolProfileContext::Parent)
            .iter()
            .any(|definition| definition.name.as_ref() == "workflow")
    );
}

#[test]
fn guards_or_a_bound_spec_also_request_workflow_mode() {
    let root = tempfile::tempdir().unwrap();
    let inherited = cli_parent_snapshot(root.path(), Config::default(), &["workflow".into()]);
    let guards_only = uds_child(root.path(), inherited.clone(), false, true);
    assert!(guards_only.is_err(), "guards alone ask for workflow mode");

    let spec = root.path().join("spec.json");
    std::fs::write(
        &spec,
        r#"{"template":{"id":"rev","label":"Rev","description":"d","steps":[{"key":"a","label":"A","phase":"review"}]}}"#,
    )
    .unwrap();
    let client = reqwest::Client::new();
    let mut stderr = String::new();
    let config = Config::default();
    let error = build_tool_runtime(ToolRuntimeBuildArgs {
        launch_extensions: true,
        swarm_context: None,
        swarm_participation: crate::infrastructure::tools::swarm_bridge::Participation::shared(),
        entrypoint: ToolEntrypoint::UdsAgent,
        profile_context: ToolRuntimeProfileContext::Child,
        base_dir: root.path(),
        config: &config,
        http_client: &client,
        web_fetch_tool: None,
        workspace: root.path().to_path_buf(),
        sandbox: crate::infrastructure::security::sandbox::Sandbox::new(Some(
            root.path().to_path_buf(),
        )),
        exec_options: Default::default(),
        session_key: "inherited-workflow-spec".into(),
        recall: crate::composition::sessions::build_retention_handles(root.path()).recall,
        spawned: true,
        parent_session_name: None,
        parent_config_path: None,
        effort_control: None,
        container_configs: None,
        kill_tool: None,
        disabled_tools: &[],
        inherited_tool_policy: Some(inherited),
        workflow: ToolRuntimeWorkflowPolicy {
            workflow_disabled: false,
            workflow_requested: false,
            workflow_guards: false,
            workflow_spec_path: Some(&spec),
            broadcast_tx: None,
            emitter_agent_id: None,
            emitter_parent_id: None,
            cwd: root.path(),
            home_dir: Some(root.path()),
        },
        stderr: &mut stderr,
        environment_registry: None,
    })
    .err()
    .expect("a bound spec asks for workflow mode");
    assert!(
        error.contains("workflow mode was requested"),
        "got: {error}"
    );
}
