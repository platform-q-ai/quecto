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
        metadata: serde_json::json!({"runtime":"docker-script"}),
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
    use quecto::application::agent_launch_backend::AgentLaunchBackend;
    let backend = quecto::application::agent_launch_backend::LocalProcessLaunchBackend;
    world.stdout = backend.backend_name().into();
    assert!(backend.can_launch(&SpawnContainerRequest::Local));
}

#[when("the parent requests a new isolated environment from that backend")]
fn parent_requests_new_environment_from_local_backend(world: &mut QuectoWorld) {
    use quecto::application::agent_launch_backend::AgentLaunchBackend;
    let backend = quecto::application::agent_launch_backend::LocalProcessLaunchBackend;
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
