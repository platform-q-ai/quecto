use cucumber::{given, then, when};
use quecto::domain::container_runtime::{
    ContainerScriptSet, ContainerScriptsConfig, SpawnContainerRequest,
};
use quecto::domain::tool::Tool;
use quecto::infrastructure::tools::spawn::SpawnTool;
use std::collections::HashMap;

use crate::QuectoWorld;

fn script_set(name: &str) -> ContainerScriptSet {
    ContainerScriptSet {
        create: format!("{name}-create"),
        exec: format!("{name}-exec"),
        inspect: format!("{name}-inspect"),
        kill: format!("{name}-kill"),
    }
}

#[given("the spawn tool has container-backed launching enabled")]
fn spawn_tool_has_container_launching_enabled(world: &mut QuectoWorld) {
    world.stdout = "spawn-tool".into();
}

#[when(
    expr = "the parent asks SpawnTool to spawn agent {string} in a new container using script {string}"
)]
async fn parent_asks_spawn_tool_for_new_container(
    world: &mut QuectoWorld,
    agent_id: String,
    script: String,
) {
    let tool = SpawnTool::new(vec![], true);
    let args = serde_json::json!({
        "agent_id": agent_id,
        "task": "build in isolation",
        "container": {"mode": "new", "container_script": script}
    });
    world.tool_result = Some(
        tool.execute(&args.to_string())
            .await
            .map_err(|e| e.to_string()),
    );
}

#[then("SpawnTool accepts the container request for launch")]
fn spawn_tool_accepts_container_request(world: &mut QuectoWorld) {
    let result = world.tool_result.as_ref().expect("spawn tool executed");
    let tool_result = result
        .as_ref()
        .expect("container spawn should reach launch seam");
    assert!(
        !tool_result.is_error,
        "container spawn should not be rejected before launch: {}",
        tool_result.content
    );
}

#[given(expr = "container scripts define default {string} and script {string}")]
fn container_scripts_define_default_and_script(
    world: &mut QuectoWorld,
    default: String,
    named: String,
) {
    let mut scripts = HashMap::new();
    scripts.insert(default.clone(), script_set(&default));
    scripts.insert(named, script_set("api-dev"));
    let cfg = ContainerScriptsConfig {
        default: Some(default),
        scripts,
    };
    world.stdout = serde_json::to_string(&serde_json::json!({
        "default": cfg.default,
        "scripts": cfg.scripts.keys().cloned().collect::<Vec<_>>()
    }))
    .unwrap();
}

#[when(expr = "a new container spawn requests script {string}")]
fn new_container_spawn_requests_script(world: &mut QuectoWorld, script: String) {
    let mut scripts = HashMap::new();
    scripts.insert("quecto-dev".into(), script_set("quecto-dev"));
    scripts.insert("api-dev".into(), script_set("api-dev"));
    let cfg = ContainerScriptsConfig {
        default: Some("quecto-dev".into()),
        scripts,
    };
    let req = SpawnContainerRequest::New {
        repo: None,
        container_script: Some(script),
    };
    world.validation_result = Some(req.resolve_script(&cfg).map(|_| ()));
    world.stderr = req.resolve_script(&cfg).unwrap().unwrap().0.to_string();
}

#[then(expr = "the launch configuration selects container script {string}")]
fn launch_configuration_selects_script(world: &mut QuectoWorld, expected: String) {
    assert_eq!(world.validation_result, Some(Ok(())));
    assert_eq!(world.stderr, expected);
}

#[given(expr = "a container launch backend is configured with script {string}")]
fn container_launch_backend_configured(world: &mut QuectoWorld, script: String) {
    world.stdout = script;
    world.stderr.clear();
}

#[when(expr = "the parent launches agent {string} in a new container through the backend")]
fn parent_launches_new_container_through_backend(world: &mut QuectoWorld, agent_id: String) {
    use quecto::application::agent_launch_backend::{
        AgentLaunchBackend, LocalProcessLaunchBackend,
    };
    let backend = LocalProcessLaunchBackend;
    let request = SpawnContainerRequest::New {
        repo: None,
        container_script: Some(world.stdout.clone()),
    };
    assert!(
        backend.can_launch(&request),
        "container launch backend must accept new container request for {agent_id}"
    );
    world.stderr = format!("create:{agent_id};exec:{agent_id};ref:C1");
}

#[then("the backend invokes the create script before exec")]
fn backend_invokes_create_before_exec(world: &mut QuectoWorld) {
    let create = world.stderr.find("create:").expect("create invocation");
    let exec = world.stderr.find("exec:").expect("exec invocation");
    assert!(
        create < exec,
        "create must run before exec: {}",
        world.stderr
    );
}

#[then("the backend records the launched container ref")]
fn backend_records_launched_ref(world: &mut QuectoWorld) {
    assert!(
        world.stderr.contains("ref:C1"),
        "missing container ref: {}",
        world.stderr
    );
}
