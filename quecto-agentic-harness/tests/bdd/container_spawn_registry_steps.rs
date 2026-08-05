use cucumber::{given, then, when};
use quecto::infrastructure::tools::container_registry::{
    ContainerEntry, ContainerStatus, add_agent_to_live_container, list_containers,
    new_container_registry, register_container, resolve_live_ref,
};
use serde_json::{Value, json};

use crate::QuectoWorld;

fn entry(
    reference: &str,
    repo: &str,
    status: ContainerStatus,
    agents: Vec<&str>,
) -> ContainerEntry {
    ContainerEntry {
        container_uuid: format!("env-uuid-{}", reference.trim_start_matches('C')),
        container_ref: reference.into(),
        container_name: Some(reference.into()),
        environment_id: format!("env-{}", reference),
        repo_url: Some(repo.into()),
        workspace_path: "/workspace/quecto".into(),
        status,
        agents: agents.into_iter().map(Into::into).collect(),
        script_name: "dev".into(),
        exec_command: "true".into(),
        inspect_command: "true".into(),
        kill_command: "true".into(),
        socket_path: Some(format!("/tmp/{reference}.sock")),
        socket_proxy: None,
        metadata: json!({"bdd":"production-registry"}),
    }
}

fn save_entries(world: &mut QuectoWorld, entries: Vec<ContainerEntry>) {
    world.stdout = serde_json::to_string(&entries).unwrap();
}

fn load_registry(
    world: &QuectoWorld,
) -> quecto::infrastructure::tools::container_registry::ContainerRegistry {
    let registry = new_container_registry();
    let entries: Vec<ContainerEntry> = serde_json::from_str(&world.stdout).unwrap_or_default();
    for e in entries {
        register_container(&registry, e);
    }
    registry
}

fn save_registry(
    world: &mut QuectoWorld,
    registry: &quecto::infrastructure::tools::container_registry::ContainerRegistry,
) {
    save_entries(world, list_containers(registry));
}

fn status(world: &QuectoWorld) -> Value {
    serde_json::from_str(&world.stderr).unwrap_or_else(|_| json!({}))
}

#[given(expr = "a parent session has created container ref {string} for repository {string}")]
fn parent_session_created_container_ref(world: &mut QuectoWorld, reference: String, repo: String) {
    let registry = new_container_registry();
    register_container(
        &registry,
        entry(
            &reference,
            &repo,
            ContainerStatus::Running,
            vec!["agent-impl-1"],
        ),
    );
    save_registry(world, &registry);
    world.stderr = json!({"last_created_ref": reference}).to_string();
}

#[given(expr = "container ref {string} has stopped")]
fn container_ref_has_stopped(world: &mut QuectoWorld, reference: String) {
    let registry = new_container_registry();
    let mut entries: Vec<ContainerEntry> = serde_json::from_str(&world.stdout).unwrap_or_default();
    for e in &mut entries {
        if e.container_ref == reference {
            e.status = ContainerStatus::Stopped;
        }
        register_container(&registry, e.clone());
    }
    save_registry(world, &registry);
}

#[given(expr = "the parent has spawned an implementer and observer in container ref {string}")]
fn parent_has_spawned_implementer_and_observer(world: &mut QuectoWorld, reference: String) {
    let registry = load_registry(world);
    let uuid = resolve_live_ref(&registry, &reference).unwrap();
    add_agent_to_live_container(&registry, &uuid, "agent-obs-1".into()).unwrap();
    save_registry(world, &registry);
}

#[when(expr = "the parent spawns a read-only observer into existing container ref {string}")]
fn parent_spawns_readonly_observer_existing_ref(world: &mut QuectoWorld, reference: String) {
    let registry = load_registry(world);
    let result = resolve_live_ref(&registry, &reference).and_then(|uuid| {
        add_agent_to_live_container(&registry, &uuid, "agent-obs-1".into()).map(|_| uuid)
    });
    world.stderr = match result {
        Ok(uuid) => json!({"targeted": reference, "resolved_uuid": uuid, "last_spawn_error": null})
            .to_string(),
        Err(e) => json!({"targeted": null, "last_spawn_error": e}).to_string(),
    };
    save_registry(world, &registry);
}

#[when(expr = "the parent spawns an agent into existing container ref {string}")]
fn parent_spawns_agent_existing_ref(world: &mut QuectoWorld, reference: String) {
    let registry = load_registry(world);
    let result = resolve_live_ref(&registry, &reference).and_then(|uuid| {
        add_agent_to_live_container(&registry, &uuid, "agent-new".into()).map(|_| uuid)
    });
    world.stderr = match result {
        Ok(_) => json!({"targeted": reference, "last_spawn_error": null}).to_string(),
        Err(e) => json!({"targeted": null, "last_spawn_error": e}).to_string(),
    };
    save_registry(world, &registry);
}

#[when(expr = "the parent creates another container for repository {string}")]
fn parent_creates_another_container(world: &mut QuectoWorld, repo: String) {
    let registry = load_registry(world);
    let next = list_containers(&registry).len() + 1;
    let reference = format!("C{next}");
    register_container(
        &registry,
        entry(&reference, &repo, ContainerStatus::Running, vec![]),
    );
    world.stderr = json!({"last_created_ref": reference}).to_string();
    save_registry(world, &registry);
}

#[when("the parent requests the container list through the agent protocol")]
fn parent_requests_container_list_through_protocol(world: &mut QuectoWorld) {
    let registry = load_registry(world);
    world.stderr = serde_json::to_string(&list_containers(&registry)).unwrap();
}

#[then(expr = "the observer is accepted into container ref {string}")]
fn observer_accepted_into_container_ref(world: &mut QuectoWorld, reference: String) {
    let registry = load_registry(world);
    let containers = list_containers(&registry);
    let c = containers
        .iter()
        .find(|c| c.container_ref == reference)
        .unwrap();
    assert!(c.agents.iter().any(|a| a.as_ref() == "agent-obs-1"));
}

#[then("the observer workspace path matches the implementing agent workspace path")]
fn observer_workspace_matches_implementer(world: &mut QuectoWorld) {
    let registry = load_registry(world);
    let c = list_containers(&registry).into_iter().next().unwrap();
    assert!(c.agents.iter().any(|a| a.as_ref() == "agent-impl-1"));
    assert!(c.agents.iter().any(|a| a.as_ref() == "agent-obs-1"));
    assert_eq!(c.workspace_path, "/workspace/quecto");
}

#[then(expr = "the spawn fails because container ref {string} is unknown")]
fn spawn_fails_unknown_ref(world: &mut QuectoWorld, reference: String) {
    assert!(
        status(world)["last_spawn_error"]
            .as_str()
            .unwrap_or("")
            .contains(&reference)
    );
}

#[then(expr = "the spawn fails because container ref {string} is not live")]
fn spawn_fails_dead_ref(world: &mut QuectoWorld, reference: String) {
    let msg = status(world)["last_spawn_error"]
        .as_str()
        .unwrap_or("")
        .to_string();
    assert!(msg.contains(&reference) && msg.contains("not live"));
}

#[then("no other container is targeted")]
fn no_other_container_is_targeted(world: &mut QuectoWorld) {
    assert!(status(world)["targeted"].is_null());
}

#[then(expr = "the new container ref is {string}")]
fn new_container_ref_is(world: &mut QuectoWorld, expected: String) {
    assert_eq!(status(world)["last_created_ref"], expected);
}

#[then(expr = "the container list includes ref {string}")]
fn container_list_includes_ref(world: &mut QuectoWorld, reference: String) {
    let list: Vec<ContainerEntry> = serde_json::from_str(&world.stderr).unwrap();
    assert!(list.iter().any(|c| c.container_ref == reference));
}

#[then(expr = "the container list includes repository {string}")]
fn container_list_includes_repository(world: &mut QuectoWorld, repo: String) {
    let list: Vec<ContainerEntry> = serde_json::from_str(&world.stderr).unwrap();
    assert!(
        list.iter()
            .any(|c| c.repo_url.as_deref() == Some(repo.as_str()))
    );
}

#[then("the container list includes the implementer and observer members")]
fn container_list_includes_implementer_and_observer(world: &mut QuectoWorld) {
    let list: Vec<ContainerEntry> = serde_json::from_str(&world.stderr).unwrap();
    let c = &list[0];
    assert!(c.agents.iter().any(|a| a.as_ref() == "agent-impl-1"));
    assert!(c.agents.iter().any(|a| a.as_ref() == "agent-obs-1"));
}

#[then("the container uuid is not the implementer agent uuid")]
fn container_uuid_not_implementer_uuid(world: &mut QuectoWorld) {
    let list: Vec<ContainerEntry> = serde_json::from_str(&world.stderr).unwrap();
    assert_ne!(list[0].container_uuid, "agent-impl-1");
}

#[then("the container uuid is not the observer agent uuid")]
fn container_uuid_not_observer_uuid(world: &mut QuectoWorld) {
    let list: Vec<ContainerEntry> = serde_json::from_str(&world.stderr).unwrap();
    assert_ne!(list[0].container_uuid, "agent-obs-1");
}

#[then(expr = "the implementer and observer have workspace path {string}")]
fn implementer_and_observer_have_workspace_path(world: &mut QuectoWorld, workspace: String) {
    let list: Vec<ContainerEntry> = serde_json::from_str(&world.stderr).unwrap();
    let c = &list[0];
    assert_eq!(c.workspace_path, workspace);
    assert!(c.agents.iter().any(|a| a.as_ref() == "agent-impl-1"));
    assert!(c.agents.iter().any(|a| a.as_ref() == "agent-obs-1"));
}
