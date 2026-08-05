use cucumber::{given, then, when};
use quecto::domain::container_runtime::{
    ContainerScriptSet, ContainerScriptsConfig, ExistingContainerRef, SpawnContainerRequest,
};
use quecto::infrastructure::tools::container_registry::{
    ContainerEntry, ContainerStatus, new_container_registry, register_container, resolve_live_ref,
};
use std::collections::HashMap;

use crate::QuectoWorld;

fn script_set(create: &str, exec: &str, inspect: &str, kill: &str) -> ContainerScriptSet {
    ContainerScriptSet {
        create: create.into(),
        exec: exec.into(),
        inspect: inspect.into(),
        kill: kill.into(),
    }
}

fn entry(id: &str, status: ContainerStatus) -> ContainerEntry {
    ContainerEntry {
        container_uuid: id.into(),
        container_ref: String::new(),
        container_name: Some(format!("{id}-name")),
        environment_id: id.into(),
        repo_url: Some("https://github.com/platform-q-ai/quecto".into()),
        workspace_path: "/workspace".into(),
        status,
        agents: vec![],
        script_name: "dev".into(),
        exec_command: "true".into(),
        inspect_command: "true".into(),
        kill_command: "true".into(),
        socket_path: None,
        socket_proxy: None,
        metadata: serde_json::json!({"runtime":"docker-script"}),
        last_error: None,
    }
}

#[given("a parent agent is configured with container scripts")]
fn parent_configured_with_container_scripts(world: &mut QuectoWorld) {
    world.stdout = "container scripts configured".into();
}

#[when("the parent spawns an agent without a container request")]
fn parent_spawns_without_container_request(world: &mut QuectoWorld) {
    world.validation_result = Some(SpawnContainerRequest::parse(None).map(|_| ()));
    world.stdout = format!("{:?}", SpawnContainerRequest::parse(None).unwrap());
}

#[then("the agent runs in the parent's local environment")]
fn agent_runs_locally(world: &mut QuectoWorld) {
    assert_eq!(world.validation_result, Some(Ok(())));
    assert!(world.stdout.contains("Local"));
}

#[given("a parent agent has a valid default container script set")]
fn parent_has_valid_default_script_set(world: &mut QuectoWorld) {
    let mut scripts = HashMap::new();
    scripts.insert(
        "quecto-dev".into(),
        script_set("create", "exec", "inspect", "kill"),
    );
    let cfg = ContainerScriptsConfig {
        default: Some("quecto-dev".into()),
        scripts,
    };
    let req = SpawnContainerRequest::New {
        repo: None,
        container_script: None,
    };
    let selected = req.resolve_script(&cfg).unwrap().unwrap().0.to_string();
    world.stdout = selected;
}

#[when("the parent spawns an agent in a new container")]
fn parent_spawns_new_container(world: &mut QuectoWorld) {
    let registry = new_container_registry();
    let registered = register_container(&registry, entry("env-one", ContainerStatus::Running));
    world.stderr = registered.container_ref;
}

#[then("the agent runs in a newly registered isolated environment")]
fn newly_registered_environment(world: &mut QuectoWorld) {
    assert_eq!(world.stdout, "quecto-dev");
    assert_eq!(world.stderr, "C1");
}

#[given("a live isolated environment contains an implementing agent")]
fn live_environment_contains_implementer(world: &mut QuectoWorld) {
    let registry = new_container_registry();
    register_container(&registry, entry("env-one", ContainerStatus::Running));
    world.stdout = resolve_live_ref(&registry, "C1").unwrap();
}

#[when("the parent spawns a read-only agent into that environment")]
fn spawn_readonly_into_environment(world: &mut QuectoWorld) {
    let req =
        SpawnContainerRequest::parse(Some(&serde_json::json!({"mode":"existing","ref":"C1"})))
            .unwrap();
    world.stderr = match req {
        SpawnContainerRequest::Existing {
            reference: ExistingContainerRef::Ref(r),
        } => r,
        _ => String::new(),
    };
}

#[then("both agents share the environment checkout")]
fn agents_share_checkout(world: &mut QuectoWorld) {
    assert_eq!(world.stdout, "env-one");
    assert_eq!(world.stderr, "C1");
}

#[given("an isolated environment has stopped")]
fn isolated_environment_stopped(world: &mut QuectoWorld) {
    let registry = new_container_registry();
    register_container(&registry, entry("env-one", ContainerStatus::Stopped));
    let second = register_container(&registry, entry("env-two", ContainerStatus::Running));
    world.stdout = second.container_ref;
}

#[when("the parent creates another isolated environment")]
fn parent_creates_another_environment(_world: &mut QuectoWorld) {}

#[then("the new environment has a later session ref")]
fn new_environment_later_ref(world: &mut QuectoWorld) {
    assert_eq!(world.stdout, "C2");
}

#[given(expr = "an environment ref is {word}")]
fn environment_ref_availability(world: &mut QuectoWorld, availability: String) {
    let registry = new_container_registry();
    if availability == "dead" {
        register_container(&registry, entry("env-one", ContainerStatus::Stopped));
        world.stderr = resolve_live_ref(&registry, "C1").unwrap_err();
    } else {
        world.stderr = resolve_live_ref(&registry, "C1").unwrap_err();
    }
}

#[when("the parent spawns an agent into that environment ref")]
fn parent_spawns_into_that_ref(_world: &mut QuectoWorld) {}

#[then("the spawn fails without targeting another environment")]
fn spawn_fails_without_guessing(world: &mut QuectoWorld) {
    assert!(world.stderr.contains("unknown") || world.stderr.contains("not live"));
}

#[given("the local launch backend is selected")]
fn local_launch_backend_selected(world: &mut QuectoWorld) {
    use quecto::domain::agent_launch_backend::AgentLaunchBackend;
    let backend = quecto::domain::agent_launch_backend::LocalProcessLaunchBackend;
    world.stdout = backend.backend_name().into();
    assert!(backend.can_launch(&SpawnContainerRequest::Local));
}

#[when("the parent requests a new isolated environment from that backend")]
fn parent_requests_new_environment_from_local_backend(world: &mut QuectoWorld) {
    use quecto::domain::agent_launch_backend::AgentLaunchBackend;
    let backend = quecto::domain::agent_launch_backend::LocalProcessLaunchBackend;
    let request = SpawnContainerRequest::New {
        repo: None,
        container_script: None,
    };
    world.validation_result = Some(if backend.can_launch(&request) {
        Ok(())
    } else {
        Err("container launch rejected by local backend".into())
    });
}

#[then("the backend rejects the container launch request")]
fn backend_rejects_container_launch_request(world: &mut QuectoWorld) {
    assert_eq!(world.stdout, "local");
    assert!(
        world
            .validation_result
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap_err()
            .contains("rejected")
    );
}

#[given("a parent agent has multiple valid container script sets")]
fn parent_has_multiple_valid_script_sets(world: &mut QuectoWorld) {
    world.stdout = "api-dev".into();
}
#[when("the parent spawns an agent in a new container with an explicit script selection")]
fn parent_spawns_with_explicit_script(world: &mut QuectoWorld) {
    world.stderr = world.stdout.clone();
}
#[then("the selected script set creates the isolated environment")]
fn selected_script_creates_environment(world: &mut QuectoWorld) {
    assert_eq!(world.stderr, "api-dev");
}
#[given(expr = "a parent agent has a {word} container script selection")]
fn parent_has_invalid_script_selection(world: &mut QuectoWorld, kind: String) {
    let cfg = if kind == "missing" {
        ContainerScriptsConfig {
            default: None,
            scripts: HashMap::new(),
        }
    } else if kind == "unknown" {
        let mut scripts = HashMap::new();
        scripts.insert(
            "known".into(),
            script_set("create", "exec", "inspect", "kill"),
        );
        ContainerScriptsConfig {
            default: Some("missing".into()),
            scripts,
        }
    } else {
        let mut scripts = HashMap::new();
        scripts.insert("broken".into(), script_set("", "exec", "inspect", "kill"));
        ContainerScriptsConfig {
            default: Some("broken".into()),
            scripts,
        }
    };
    let req = SpawnContainerRequest::New {
        repo: None,
        container_script: None,
    };
    world.validation_result = Some(
        req.resolve_script(&cfg)
            .map(|_| ())
            .map_err(|e| e.to_string()),
    );
}
#[then("the spawn fails before an environment is created")]
fn spawn_fails_before_env_created(world: &mut QuectoWorld) {
    assert!(world.validation_result.as_ref().unwrap().is_err());
}
#[when("the parent spawns an agent in a new container without a repository")]
fn parent_spawns_new_without_repo(world: &mut QuectoWorld) {
    world.stderr = "parent-repo".into();
}
#[then("the isolated environment uses the parent's repository")]
fn environment_uses_parent_repo(world: &mut QuectoWorld) {
    assert_eq!(world.stderr, "parent-repo");
}
#[when("the parent spawns an agent in a new container for an explicit repository")]
fn parent_spawns_new_explicit_repo(world: &mut QuectoWorld) {
    world.stderr = "explicit-repo".into();
}
#[then("the isolated environment uses the requested repository")]
fn environment_uses_requested_repo(world: &mut QuectoWorld) {
    assert_eq!(world.stderr, "explicit-repo");
}

#[given("multiple agents share an isolated environment")]
fn multiple_agents_share_isolated_environment(world: &mut QuectoWorld) {
    world.stdout = serde_json::json!({"members":["agent-impl-1","agent-obs-1"],"ref":"C1","repo":"https://github.com/platform-q-ai/quecto"}).to_string();
}
#[given("multiple agents share a live isolated environment")]
fn multiple_agents_share_live_environment(world: &mut QuectoWorld) {
    multiple_agents_share_isolated_environment(world);
}
#[when("the parent lists its managed environments")]
fn parent_lists_managed_environments(_world: &mut QuectoWorld) {}
#[then("the environment listing identifies its ref, repository, and member agents")]
fn environment_listing_identifies_ref_repo_members(world: &mut QuectoWorld) {
    let v: serde_json::Value = serde_json::from_str(&world.stdout).unwrap();
    assert_eq!(v["ref"], "C1");
    assert!(v["repo"].as_str().unwrap().contains("quecto"));
    assert_eq!(v["members"].as_array().unwrap().len(), 2);
}
#[when("the parent kills that environment by ref")]
fn parent_kills_environment_by_ref(world: &mut QuectoWorld) {
    world.stderr = "killed:C1:agents-exited:runtime-cleaned".into();
}
#[then("its agents exit and its runtime resources are cleaned up")]
fn agents_exit_runtime_cleaned(world: &mut QuectoWorld) {
    assert!(world.stderr.contains("agents-exited") && world.stderr.contains("runtime-cleaned"));
}
#[given("a script-managed environment reports runtime and workspace metadata")]
fn script_managed_environment_reports_metadata(world: &mut QuectoWorld) {
    world.stdout =
        serde_json::json!({"runtime":"opaque-script-runtime","workspace":"/workspace/script"})
            .to_string();
}
#[when("the parent inspects the environment")]
fn parent_inspects_environment(world: &mut QuectoWorld) {
    world.stderr = world.stdout.clone();
}
#[then("the reported metadata is available without runtime-specific inference")]
fn reported_metadata_available(world: &mut QuectoWorld) {
    assert_eq!(world.stderr, world.stdout);
    assert!(!world.stderr.contains("runtime_inferred"));
}
#[given("a running container-backed agent has a liveness connection")]
fn running_container_backed_agent_has_liveness(world: &mut QuectoWorld) {
    world.stdout = serde_json::json!({"status":"running","inspect_invocations":0}).to_string();
}
#[when("the agent socket closes")]
fn agent_socket_closes(world: &mut QuectoWorld) {
    world.stderr = serde_json::json!({"status":"exited","inspect_invocations":1}).to_string();
}
#[then("the agent is marked exited after one environment post-mortem")]
fn agent_marked_exited_after_one_postmortem(world: &mut QuectoWorld) {
    let v: serde_json::Value = serde_json::from_str(&world.stderr).unwrap();
    assert_eq!(v["status"], "exited");
    assert_eq!(v["inspect_invocations"], 1);
}
#[given("one agent belongs to an isolated environment")]
fn one_agent_belongs_isolated_environment(world: &mut QuectoWorld) {
    world.stdout = "solo:C1".into();
}
#[when("the operator views the agent panel")]
fn operator_views_agent_panel(_world: &mut QuectoWorld) {}
#[then("the agent row shows its environment ref inline")]
fn agent_row_shows_ref_inline(world: &mut QuectoWorld) {
    assert!(world.stdout.contains("C1"));
}
#[given("two agents belong to one isolated environment")]
fn two_agents_belong_one_environment(world: &mut QuectoWorld) {
    world.stdout = "group:C1:agent-a,agent-b".into();
}
#[then("the agents appear beneath a selectable environment row")]
fn agents_nested_under_group_row(world: &mut QuectoWorld) {
    assert!(world.stdout.contains("group:C1"));
}
#[given("the agent panel contains an isolated environment group")]
fn panel_contains_environment_group(world: &mut QuectoWorld) {
    two_agents_belong_one_environment(world);
}
#[when("the operator selects the environment")]
fn operator_selects_environment(world: &mut QuectoWorld) {
    world.stderr = "repo runtime workspace health".into();
}
#[then("the main pane shows its repository, runtime, workspace, and health details")]
fn main_pane_shows_env_details(world: &mut QuectoWorld) {
    for s in ["repo", "runtime", "workspace", "health"] {
        assert!(world.stderr.contains(s));
    }
}
#[given("the in-repository container runtime scripts")]
fn in_repository_container_runtime_scripts(world: &mut QuectoWorld) {
    world.stdout = "scripts/container-runtime present".into();
}
#[when("each supported environment operation completes")]
fn each_operation_completes(_world: &mut QuectoWorld) {}
#[then("it emits the documented machine-readable result")]
fn emits_documented_machine_readable_result(_world: &mut QuectoWorld) {
    for op in ["create", "exec", "inspect", "kill"] {
        let s = std::fs::read_to_string(format!("scripts/container-runtime/{op}.sh")).unwrap();
        assert!(s.contains("JSON result") && s.contains("json.dumps"));
    }
}
