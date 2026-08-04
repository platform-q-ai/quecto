use cucumber::{given, then, when};
use serde_json::{Value, json};

use crate::QuectoWorld;

fn state(world: &QuectoWorld) -> Value {
    serde_json::from_str(&world.stdout).unwrap_or_else(|_| json!({}))
}

fn put_state(world: &mut QuectoWorld, value: Value) {
    world.stdout = value.to_string();
}

fn ensure_state(world: &mut QuectoWorld) -> Value {
    let current = state(world);
    if current.is_object() && current.get("containers").is_some() {
        current
    } else {
        json!({"next_ref":1,"targeted":null,"last_spawn_error":null,"last_created_ref":null,"protocol_containers":null,"containers":[]})
    }
}

fn container<'a>(state: &'a Value, reference: &str) -> Option<&'a Value> {
    state["containers"]
        .as_array()?
        .iter()
        .find(|c| c["ref"] == reference)
}

#[given(expr = "a parent session has created container ref {string} for repository {string}")]
fn parent_session_created_container_ref(world: &mut QuectoWorld, reference: String, repo: String) {
    let mut s = ensure_state(world);
    s["containers"] = json!([{"ref":reference,"container_uuid":"env-uuid-1","repo":repo,"status":"running","workspace_path":"/workspace/quecto","members":[{"role":"implementer","agent_uuid":"agent-impl-1","workspace_path":"/workspace/quecto"}]}]);
    s["next_ref"] = json!(2);
    s["last_created_ref"] = json!("C1");
    put_state(world, s);
}

#[given(expr = "container ref {string} has stopped")]
fn container_ref_has_stopped(world: &mut QuectoWorld, reference: String) {
    let mut s = ensure_state(world);
    if let Some(items) = s["containers"].as_array_mut() {
        if let Some(item) = items.iter_mut().find(|c| c["ref"] == reference) {
            item["status"] = json!("stopped");
        }
    }
    put_state(world, s);
}

#[given(expr = "the parent has spawned an implementer and observer in container ref {string}")]
fn parent_has_spawned_implementer_and_observer(world: &mut QuectoWorld, reference: String) {
    let mut s = ensure_state(world);
    if let Some(items) = s["containers"].as_array_mut() {
        if let Some(item) = items.iter_mut().find(|c| c["ref"] == reference) {
            item["members"] = json!([{"role":"implementer","agent_uuid":"agent-impl-1","workspace_path":"/workspace/quecto"}]);
        }
    }
    put_state(world, s);
}

#[when(expr = "the parent spawns a read-only observer into existing container ref {string}")]
fn parent_spawns_readonly_observer_existing_ref(world: &mut QuectoWorld, reference: String) {
    let mut s = ensure_state(world);
    s["targeted"] = json!(reference);
    s["last_spawn_error"] = json!("existing container join is not implemented");
    put_state(world, s);
}

#[when(expr = "the parent spawns an agent into existing container ref {string}")]
fn parent_spawns_agent_existing_ref(world: &mut QuectoWorld, reference: String) {
    let mut s = ensure_state(world);
    match container(&s, &reference).and_then(|c| c["status"].as_str()) {
        None => {
            s["targeted"] = Value::Null;
            s["last_spawn_error"] = json!(format!("container ref {reference} is unknown"));
        }
        Some("running") => {
            s["targeted"] = json!(reference);
            s["last_spawn_error"] = Value::Null;
        }
        Some(_) => {
            s["targeted"] = Value::Null;
            s["last_spawn_error"] = json!(format!("container ref {reference} is not live"));
        }
    }
    put_state(world, s);
}

#[when(expr = "the parent creates another container for repository {string}")]
fn parent_creates_another_container(world: &mut QuectoWorld, repo: String) {
    let mut s = ensure_state(world);
    let next = s["next_ref"].as_u64().unwrap_or(1);
    let reference = format!("C{next}");
    s["next_ref"] = json!(next + 1);
    s["last_created_ref"] = json!(reference.clone());
    s["containers"].as_array_mut().unwrap().push(json!({"ref":reference,"container_uuid":format!("env-uuid-{next}"),"repo":repo,"status":"running","workspace_path":"/workspace/quecto","members":[]}));
    put_state(world, s);
}

#[when("the parent requests the container list through the agent protocol")]
fn parent_requests_container_list_through_protocol(world: &mut QuectoWorld) {
    let mut s = ensure_state(world);
    s["protocol_containers"] = Value::Null;
    put_state(world, s);
}

#[then(expr = "the observer is accepted into container ref {string}")]
fn observer_accepted_into_container_ref(world: &mut QuectoWorld, reference: String) {
    let s = state(world);
    let c = container(&s, &reference).expect("container ref should exist");
    assert!(
        c["members"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["role"] == "observer"),
        "observer was not joined into existing container ref {reference}"
    );
}

#[then("the observer workspace path matches the implementing agent workspace path")]
fn observer_workspace_matches_implementer(world: &mut QuectoWorld) {
    let s = state(world);
    let members = s["containers"][0]["members"].as_array().unwrap();
    let implementer = members.iter().find(|m| m["role"] == "implementer").unwrap();
    let observer = members.iter().find(|m| m["role"] == "observer").unwrap();
    assert_eq!(observer["workspace_path"], implementer["workspace_path"]);
}

#[then(expr = "the spawn fails because container ref {string} is unknown")]
fn spawn_fails_unknown_ref(world: &mut QuectoWorld, reference: String) {
    assert!(
        state(world)["last_spawn_error"]
            .as_str()
            .unwrap_or("")
            .contains(&format!("{reference} is unknown"))
    );
}

#[then(expr = "the spawn fails because container ref {string} is not live")]
fn spawn_fails_dead_ref(world: &mut QuectoWorld, reference: String) {
    assert!(
        state(world)["last_spawn_error"]
            .as_str()
            .unwrap_or("")
            .contains(&format!("{reference} is not live"))
    );
}

#[then("no other container is targeted")]
fn no_other_container_is_targeted(world: &mut QuectoWorld) {
    assert!(state(world)["targeted"].is_null());
}

#[then(expr = "the new container ref is {string}")]
fn new_container_ref_is(world: &mut QuectoWorld, expected: String) {
    assert_eq!(state(world)["last_created_ref"], expected);
}

#[then(expr = "the container list includes ref {string}")]
fn container_list_includes_ref(world: &mut QuectoWorld, reference: String) {
    let s = state(world);
    let list = s["protocol_containers"]
        .as_array()
        .expect("get_containers protocol response is missing");
    assert!(list.iter().any(|c| c["ref"] == reference));
}

#[then(expr = "the container list includes repository {string}")]
fn container_list_includes_repository(world: &mut QuectoWorld, repo: String) {
    let s = state(world);
    let list = s["protocol_containers"]
        .as_array()
        .expect("get_containers protocol response is missing");
    assert!(list.iter().any(|c| c["repo"] == repo));
}

#[then("the container list includes the implementer and observer members")]
fn container_list_includes_implementer_and_observer(world: &mut QuectoWorld) {
    let s = state(world);
    let list = s["protocol_containers"]
        .as_array()
        .expect("get_containers protocol response is missing");
    let members = list[0]["members"].as_array().unwrap();
    assert!(members.iter().any(|m| m["role"] == "implementer"));
    assert!(members.iter().any(|m| m["role"] == "observer"));
}

#[then("the container uuid is not the implementer agent uuid")]
fn container_uuid_not_implementer_uuid(world: &mut QuectoWorld) {
    let s = state(world);
    let list = s["protocol_containers"]
        .as_array()
        .expect("get_containers protocol response is missing");
    let c = &list[0];
    let implementer = c["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "implementer")
        .unwrap();
    assert_ne!(c["container_uuid"], implementer["agent_uuid"]);
}

#[then("the container uuid is not the observer agent uuid")]
fn container_uuid_not_observer_uuid(world: &mut QuectoWorld) {
    let s = state(world);
    let list = s["protocol_containers"]
        .as_array()
        .expect("get_containers protocol response is missing");
    let c = &list[0];
    let observer = c["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "observer")
        .unwrap();
    assert_ne!(c["container_uuid"], observer["agent_uuid"]);
}

#[then(expr = "the implementer and observer have workspace path {string}")]
fn implementer_and_observer_have_workspace_path(world: &mut QuectoWorld, workspace: String) {
    let s = state(world);
    let list = s["protocol_containers"]
        .as_array()
        .expect("get_containers protocol response is missing");
    let members = list[0]["members"].as_array().unwrap();
    assert!(
        members
            .iter()
            .any(|m| m["role"] == "implementer" && m["workspace_path"] == workspace)
    );
    assert!(
        members
            .iter()
            .any(|m| m["role"] == "observer" && m["workspace_path"] == workspace)
    );
}
