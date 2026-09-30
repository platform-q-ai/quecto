//! #2216 review: a parent's workflow denial reaches its children on every
//! path. The denial survives the agent loop's snapshot refreshes (reload,
//! live policy, turn-boundary drain). A parent that withholds workflow
//! (`--no-workflow`, a swarm member) closes it to children, as before the
//! entrypoint-only exemption. A tool claiming the bundled workflow's stable
//! id can neither register over UDS nor displace a recorded denial.
use std::collections::BTreeMap;

use super::inherited_workflow_tests::{
    Launch, build, child_sees_workflow, cli_parent_snapshot, uds_child,
};
use crate::domain::tool::{ToolPolicyApplyMode, ToolPolicyMutation, ToolProfileContext};
use crate::domain::tool_descriptor::ProfileAvailabilityScope;
use crate::infrastructure::config::Config;
use crate::infrastructure::tools::inherited_tool_policy::{
    InheritedToolPolicySnapshot, workflow_tool_identity,
};
use crate::interface::tool_runtime::{
    ToolEntrypoint, ToolRuntimeBuild, ToolRuntimeBuildArgs, ToolRuntimeProfileContext,
    ToolRuntimeWorkflowPolicy, build_tool_runtime,
};

/// A top-level UDS agent started with `--no-workflow`, optionally joined to a
/// swarm run.
fn uds_parent_without_workflow(
    root: &std::path::Path,
    disabled_tools: &[String],
    swarm_context: Option<crate::infrastructure::tools::swarm_bridge::SwarmContext>,
    workflow_disabled: bool,
) -> ToolRuntimeBuild {
    parent_in(
        ToolEntrypoint::UdsAgent,
        root,
        disabled_tools,
        swarm_context,
        workflow_disabled,
    )
}

fn parent_in(
    entrypoint: ToolEntrypoint,
    root: &std::path::Path,
    disabled_tools: &[String],
    swarm_context: Option<crate::infrastructure::tools::swarm_bridge::SwarmContext>,
    workflow_disabled: bool,
) -> ToolRuntimeBuild {
    let client = reqwest::Client::new();
    let mut stderr = String::new();
    let config = Config::default();
    build_tool_runtime(ToolRuntimeBuildArgs {
        swarm_context,
        swarm_participation: crate::infrastructure::tools::swarm_bridge::Participation::shared(),
        entrypoint,
        profile_context: ToolRuntimeProfileContext::Parent,
        base_dir: root,
        config: &config,
        http_client: &client,
        web_fetch_tool: None,
        workspace: root.to_path_buf(),
        sandbox: crate::infrastructure::security::sandbox::Sandbox::new(Some(root.to_path_buf())),
        exec_options: Default::default(),
        session_key: "workflow-denial".into(),
        recall: crate::composition::sessions::build_retention_handles(root).recall,
        spawned: false,
        parent_session_name: None,
        parent_config_path: None,
        effort_control: None,
        container_configs: None,
        kill_tool: None,
        disabled_tools,
        inherited_tool_policy: None,
        workflow: ToolRuntimeWorkflowPolicy {
            workflow_disabled,
            workflow_requested: false,
            workflow_guards: false,
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
    .expect("parent builds")
}

fn cli_parent(root: &std::path::Path, disabled_tools: &[String]) -> ToolRuntimeBuild {
    build(
        root,
        Launch {
            entrypoint: ToolEntrypoint::CliAgent,
            profile: ToolRuntimeProfileContext::Parent,
            config: Config::default(),
            disabled_tools,
            inherited: None,
            workflow_requested: false,
            workflow_guards: false,
        },
    )
    .expect("CLI parent builds")
}

fn spawn_snapshot(
    spawn: &std::sync::Arc<dyn crate::application::tools::ports::Tool>,
) -> BTreeMap<String, ProfileAvailabilityScope> {
    spawn
        .inherited_child_policy_snapshot_for_spawn()
        .expect("spawn carries the inherited policy")
}

fn assert_workflow_denied(tools: &BTreeMap<String, ProfileAvailabilityScope>, when: &str) {
    let identity = workflow_tool_identity();
    assert_eq!(
        tools.get(identity.stable_id.as_ref()),
        Some(&ProfileAvailabilityScope::None),
        "{when}: stable id"
    );
    assert_eq!(
        tools.get("workflow"),
        Some(&ProfileAvailabilityScope::None),
        "{when}: name"
    );
}

fn agent_loop(built: ToolRuntimeBuild) -> crate::application::agent_loop::AgentLoopImpl {
    crate::application::agent_loop::AgentLoopImpl::new(
        crate::application::agent_loop::AgentLoopConfig {
            provider: std::sync::Arc::new(crate::interface::test_support::StubProvider),
            tool_registry: Box::new(built.registry),
            model: "m".into(),
            max_tokens: 1024,
            temperature: 0.7,
            retention: None,
            session_key: String::new(),
            context_collapse_after_tool_calls: u32::MAX,
            max_context_tokens: 190_000,
            progress_callback: None,
            streaming: false,
            effort: None,
            audit_log: None,
            pin_recent_turns: 2,
            context_collapse_after_messages: u32::MAX,
            large_result_collapse:
                crate::domain::large_result_collapse::LargeResultCollapse::DISABLED,
            model_context_window: None,
            tool_profile_context: ToolProfileContext::Parent,
        },
    )
}

#[test]
fn denial_survives_reload_live_policy_and_turn_boundary_drain() {
    let root = tempfile::tempdir().unwrap();
    let parents = [
        (
            "--disable-tool workflow",
            cli_parent(root.path(), &["workflow".into()]),
        ),
        (
            "--no-workflow",
            uds_parent_without_workflow(root.path(), &[], None, true),
        ),
    ];
    for (case, parent) in parents {
        let spawn = parent.registry.get("spawn").unwrap().clone();
        assert_workflow_denied(&spawn_snapshot(&spawn), &format!("{case}: built"));
        let mut agent = agent_loop(parent);

        agent.apply_persisted_tool_policy_entries(&std::collections::HashMap::new());
        assert_workflow_denied(&spawn_snapshot(&spawn), &format!("{case}: reload"));

        let narrow = [ToolPolicyMutation::set_scope(
            "read",
            ProfileAvailabilityScope::Parent,
            "narrow read",
        )];
        assert!(
            agent
                .request_tool_policy_mutation(&narrow, ToolPolicyApplyMode::ImmediateIfIdle)
                .is_some()
        );
        assert_workflow_denied(&spawn_snapshot(&spawn), &format!("{case}: live"));

        agent.queue_tool_policy_mutation(&[ToolPolicyMutation::set_scope(
            "read",
            ProfileAvailabilityScope::Both,
            "widen read",
        )]);
        assert!(agent.drain_tool_policy_mutations_at_boundary().is_some());
        let tools = spawn_snapshot(&spawn);
        assert_workflow_denied(&tools, &format!("{case}: drain"));
        let child = uds_child(
            root.path(),
            InheritedToolPolicySnapshot { version: 1, tools },
            true,
            false,
        );
        assert!(child.is_err(), "{case}: the workflow child refuses");
    }
}

#[test]
fn no_workflow_parent_closes_workflow_to_its_children() {
    let root = tempfile::tempdir().unwrap();
    let parent = uds_parent_without_workflow(root.path(), &[], None, true);
    assert!(parent.registry.get("workflow").is_none());
    let tools = spawn_snapshot(parent.registry.get("spawn").unwrap());
    assert_workflow_denied(&tools, "--no-workflow parent");
    let snapshot = InheritedToolPolicySnapshot { version: 1, tools };
    let plain = uds_child(root.path(), snapshot.clone(), false, false).expect("child builds");
    assert!(!child_sees_workflow(&plain));
    assert!(uds_child(root.path(), snapshot, true, false).is_err());
}

#[test]
fn swarm_member_parent_closes_workflow_to_its_children() {
    let (swarm_dir, swarm) = crate::swarm_control_fixture::context();
    let parent = uds_parent_without_workflow(swarm_dir.path(), &[], Some(swarm), false);
    assert!(
        parent.workflow_state.is_none(),
        "a swarm member builds no workflow engine"
    );
    let tools = spawn_snapshot(parent.registry.get("spawn").unwrap());
    assert_workflow_denied(&tools, "swarm member parent");

    // A one-shot swarm member too: the swarm, not the entrypoint, withholds it.
    let (swarm_dir, swarm) = crate::swarm_control_fixture::context();
    let parent = parent_in(
        ToolEntrypoint::CliAgent,
        swarm_dir.path(),
        &[],
        Some(swarm),
        true,
    );
    let tools = spawn_snapshot(parent.registry.get("spawn").unwrap());
    assert_workflow_denied(&tools, "one-shot swarm member parent");
}

#[test]
fn cli_parent_records_no_workflow_entry() {
    let root = tempfile::tempdir().unwrap();
    let tools = cli_parent_snapshot(root.path(), Config::default(), &[]).tools;
    let identity = workflow_tool_identity();
    assert!(!tools.contains_key(identity.stable_id.as_ref()));
    assert!(!tools.contains_key("workflow"));
}

#[test]
fn uds_tool_cannot_claim_the_bundled_workflow_stable_id() {
    let root = tempfile::tempdir().unwrap();
    let parent = cli_parent(root.path(), &["workflow".into()]);
    let id = workflow_tool_identity().stable_id.into_owned();
    assert!(
        !parent
            .registry
            .can_register_uds_tool_for_owner_with_stable_id("wf_spoof", "uds:client-a", Some(&id)),
        "only a bundled-native registration may use a bundled-native stable id"
    );
    parent
        .registry
        .refresh_spawn_inherited_child_policy_snapshot();
    let tools = spawn_snapshot(parent.registry.get("spawn").unwrap());
    assert_workflow_denied(&tools, "after the refused spoof");
}

#[test]
fn child_spawned_without_workflow_closes_it_to_grandchildren() {
    let root = tempfile::tempdir().unwrap();
    let inherited = cli_parent_snapshot(root.path(), Config::default(), &[]);
    let child = build(
        root.path(),
        Launch {
            entrypoint: ToolEntrypoint::UdsAgent,
            profile: ToolRuntimeProfileContext::Child,
            config: Config::default(),
            disabled_tools: &["workflow".into()],
            inherited: Some(inherited),
            workflow_requested: false,
            workflow_guards: false,
        },
    )
    .expect("child builds");
    let tools = spawn_snapshot(child.registry.get("spawn").unwrap());
    assert_workflow_denied(&tools, "a child spawned with disable_tools workflow");
    let grandchild = uds_child(
        root.path(),
        InheritedToolPolicySnapshot { version: 1, tools },
        false,
        false,
    )
    .expect("grandchild builds");
    assert!(!child_sees_workflow(&grandchild));
}
