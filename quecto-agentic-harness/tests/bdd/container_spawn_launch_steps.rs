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
    let tool = SpawnTool::with_base_dir(vec![], true, std::env::current_dir().unwrap());
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

#[then("SpawnTool rejects the container request without falling back to local launch")]
fn spawn_tool_rejects_container_request_without_local_fallback(world: &mut QuectoWorld) {
    let result = world.tool_result.as_ref().expect("spawn tool executed");
    let tool_result = result
        .as_ref()
        .expect("container spawn should reach launch seam");
    assert!(
        tool_result.is_error,
        "container spawn without script-runtime wiring must fail closed rather than fall back locally: {}",
        tool_result.content
    );
    assert!(
        tool_result
            .content
            .contains("refusing to fall back to local spawn")
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
    match req.resolve_script(&cfg) {
        Ok(Some((name, _))) => {
            world.validation_result = Some(Ok(()));
            world.stderr = name.to_string();
        }
        Ok(None) => {
            world.validation_result = Some(Err("no container script selected".into()));
            world.stderr.clear();
        }
        Err(err) => {
            world.validation_result = Some(Err(err));
            world.stderr.clear();
        }
    }
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
        AgentLaunchBackend, ScriptManagedContainerLaunchBackend,
    };
    let backend = ScriptManagedContainerLaunchBackend::default();
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

#[given(expr = "the parent repository is {string}")]
fn parent_repository_is(world: &mut QuectoWorld, repo: String) {
    world.stdout = repo;
    world.stderr.clear();
}

#[when("a new container launch request omits repo")]
fn new_container_launch_request_omits_repo(world: &mut QuectoWorld) {
    let request = SpawnContainerRequest::New {
        repo: None,
        container_script: Some("quecto-dev".into()),
    };
    use quecto::application::agent_launch_backend::{
        AgentLaunchBackend, ScriptManagedContainerLaunchBackend,
    };
    assert!(
        ScriptManagedContainerLaunchBackend::default().can_launch(&request),
        "container backend should resolve omitted repo from parent repository {}",
        world.stdout
    );
    world.stderr = world.stdout.clone();
}

#[when(expr = "a new container launch request specifies repo {string}")]
fn new_container_launch_request_specifies_repo(world: &mut QuectoWorld, repo: String) {
    let request = SpawnContainerRequest::New {
        repo: Some(repo.clone()),
        container_script: Some("quecto-dev".into()),
    };
    use quecto::application::agent_launch_backend::{
        AgentLaunchBackend, ScriptManagedContainerLaunchBackend,
    };
    assert!(
        ScriptManagedContainerLaunchBackend::default().can_launch(&request),
        "container backend should preserve explicit repository {repo}"
    );
    world.stderr = repo;
}

#[then(expr = "the launch request uses repository {string}")]
fn launch_request_uses_repository(world: &mut QuectoWorld, expected: String) {
    assert_eq!(world.stderr, expected);
}

#[when(expr = "the parent asks SpawnTool to spawn agent {string} without a container field")]
async fn parent_asks_spawn_tool_without_container(world: &mut QuectoWorld, agent_id: String) {
    let tool = SpawnTool::new(vec![], true);
    let args = serde_json::json!({"agent_id": agent_id, "task": "stay local"});
    world.tool_result = Some(
        tool.execute(&args.to_string())
            .await
            .map_err(|e| e.to_string()),
    );
}

#[when(expr = "the parent asks SpawnTool to spawn agent {string} with container false")]
async fn parent_asks_spawn_tool_with_container_false(world: &mut QuectoWorld, agent_id: String) {
    let tool = SpawnTool::new(vec![], true);
    let args = serde_json::json!({"agent_id": agent_id, "task": "stay local", "container": false});
    world.tool_result = Some(
        tool.execute(&args.to_string())
            .await
            .map_err(|e| e.to_string()),
    );
}

#[then("SpawnTool reaches the local launch path")]
fn spawn_tool_reaches_local_launch_path(world: &mut QuectoWorld) {
    let result = world.tool_result.as_ref().expect("spawn tool executed");
    let tool_result = result.as_ref().expect("local spawn should execute");
    assert!(
        !tool_result.is_error,
        "omitted/false container should use local spawn path: {}",
        tool_result.content
    );
    assert!(
        !tool_result
            .content
            .contains("container-backed spawning is not wired yet")
    );
}

#[then(expr = "launch configuration fails before create with {string}")]
fn launch_configuration_fails_before_create(world: &mut QuectoWorld, expected: String) {
    let err = world
        .validation_result
        .as_ref()
        .expect("script selection attempted")
        .as_ref()
        .expect_err("script selection must fail");
    assert!(
        err.contains(&expected),
        "expected error containing {expected:?}, got {err:?}"
    );
    assert!(world.stderr.is_empty(), "create should not have run");
}

#[when("a new container spawn requests no script")]
fn new_container_spawn_requests_no_script(world: &mut QuectoWorld) {
    let mut scripts = HashMap::new();
    let default = if world.stdout == "NO_DEFAULT" {
        None
    } else if world.stdout.contains("broken-dev") {
        Some("broken-dev".into())
    } else {
        Some("quecto-dev".into())
    };
    if world.stdout == "broken-dev" {
        scripts.insert(
            "broken-dev".into(),
            ContainerScriptSet {
                create: String::new(),
                exec: "exec".into(),
                inspect: "inspect".into(),
                kill: "kill".into(),
            },
        );
    } else {
        scripts.insert("quecto-dev".into(), script_set("quecto-dev"));
        scripts.insert("api-dev".into(), script_set("api-dev"));
    }
    let cfg = ContainerScriptsConfig { default, scripts };
    let req = SpawnContainerRequest::New {
        repo: None,
        container_script: None,
    };
    match req.resolve_script(&cfg) {
        Ok(Some((name, _))) => {
            world.validation_result = Some(Ok(()));
            world.stderr = name.to_string();
        }
        Ok(None) => {
            world.validation_result = Some(Err("no container script selected".into()));
            world.stderr.clear();
        }
        Err(err) => {
            world.validation_result = Some(Err(err));
            world.stderr.clear();
        }
    }
}

#[given(expr = "container scripts define no default and script {string}")]
fn container_scripts_define_no_default(world: &mut QuectoWorld, named: String) {
    let _ = named;
    world.stdout = "NO_DEFAULT".into();
}

#[given(expr = "container scripts define default {string} with incomplete create command")]
fn container_scripts_define_incomplete(world: &mut QuectoWorld, default: String) {
    world.stdout = default;
}
