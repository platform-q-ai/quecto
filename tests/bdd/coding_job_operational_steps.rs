use std::process::Command;
use std::sync::{Arc, Mutex};

use cucumber::{given, then, when};

use quecto::application::coding_coordinator::{CodingCoordinator, CoordinatorPolicy};
use quecto::domain::coding_ports::{CodingJobService, RepoValidator, SkillResolver};
use quecto::domain::tool::Tool;
use quecto::infrastructure::coding::runtime_adapters::{
    WorkspaceRepoValidator, WorkspaceSkillResolver,
};
use quecto::infrastructure::security::sandbox::Sandbox;
use quecto::infrastructure::tools::coding_job::CodingJobTool;
use quecto::infrastructure::tools::registry::ToolRegistryImpl;
use quecto::interface::shared::{self, CodingCoordinatorScopePolicy, build_coding_lifecycle};

use super::QuectoWorld;

fn init_git_repo(path: &std::path::Path) {
    std::fs::create_dir_all(path).unwrap();
    assert!(
        Command::new("git")
            .arg("init")
            .arg(path)
            .status()
            .unwrap()
            .success()
    );
    std::fs::write(path.join("README.md"), "hello\n").unwrap();
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("add")
            .arg("README.md")
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("-c")
            .arg("user.email=test@example.com")
            .arg("-c")
            .arg("user.name=test")
            .arg("commit")
            .arg("-m")
            .arg("init")
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("branch")
            .arg("-M")
            .arg("main")
            .status()
            .unwrap()
            .success()
    );
}

fn ensure_workspace_with_repo_and_skill(world: &mut QuectoWorld) -> std::path::PathBuf {
    let ws = tempfile::TempDir::new().unwrap();
    let ws_path = ws.path().to_path_buf();
    let repo = ws_path.join("test-repo");
    init_git_repo(&repo);

    let skill_dir = ws_path.join("skills").join("default-skill");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: default-skill\ndescription: default skill\n---\ncontent\n",
    )
    .unwrap();

    world.coding_operational_workspace = Some(ws_path.clone());
    world._extra_temp_dirs.push(ws);
    ws_path
}

fn exec_tool(tool: &CodingJobTool, input: &str) -> quecto::domain::tool::ToolResult {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(tool.execute(input))
        .expect("execute should not panic")
}

#[given("a workspace with a real git repository")]
fn given_workspace_with_repo(world: &mut QuectoWorld) {
    let ws = tempfile::TempDir::new().unwrap();
    let ws_path = ws.path().to_path_buf();
    let repo = ws_path.join("test-repo");
    init_git_repo(&repo);
    world.coding_operational_workspace = Some(ws_path);
    world._extra_temp_dirs.push(ws);
}

#[given("a workspace with installed skills")]
fn given_workspace_with_skills(world: &mut QuectoWorld) {
    let ws = tempfile::TempDir::new().unwrap();
    let ws_path = ws.path().to_path_buf();
    let skill_dir = ws_path.join("skills").join("default-skill");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: default-skill\ndescription: default skill\n---\ncontent\n",
    )
    .unwrap();
    world.coding_operational_workspace = Some(ws_path);
    world._extra_temp_dirs.push(ws);
}

#[given("a core tool registry for a workspace")]
fn given_core_registry(world: &mut QuectoWorld) {
    let ws = ensure_workspace_with_repo_and_skill(world);
    let sandbox = Sandbox::new(Some(ws.clone()), true);
    let registry = ToolRegistryImpl::with_core_tools(ws, sandbox);
    world.coding_operational_registry = Some(registry);
}

#[given("a workspace-backed coding_job tool")]
fn given_workspace_backed_tool(world: &mut QuectoWorld) {
    let ws = ensure_workspace_with_repo_and_skill(world);
    let repo_validator = WorkspaceRepoValidator::new(ws.clone());
    let skill_resolver = WorkspaceSkillResolver::new(ws);
    let coordinator =
        CodingCoordinator::new(repo_validator, skill_resolver, CoordinatorPolicy::default());
    let service: Arc<Mutex<dyn CodingJobService>> = Arc::new(Mutex::new(coordinator));
    world.coding_operational_tool = Some(Arc::new(CodingJobTool::new(service)));
}

#[given("a coding job exists via workspace-backed coding_job")]
fn given_job_exists_workspace_tool(world: &mut QuectoWorld) {
    let tool = world
        .coding_operational_tool
        .as_ref()
        .expect("workspace-backed tool should be initialized");
    let result = exec_tool(
        tool,
        r#"{"action":"run","goal":"test","repo":"test-repo","base_ref":"main"}"#,
    );
    assert!(!result.is_error, "run failed: {}", result.content);
    let json: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    world.coding_operational_last_job_id = Some(json["job_id"].as_str().unwrap().to_string());
}

#[when(expr = "the coding runtime validates repo {string} and base ref {string}")]
fn when_validate_repo_and_ref(world: &mut QuectoWorld, repo: String, base_ref: String) {
    let ws = world
        .coding_operational_workspace
        .as_ref()
        .expect("workspace should exist")
        .clone();
    let validator = WorkspaceRepoValidator::new(ws);
    world.coding_operational_repo_ok = validator.repo_exists(&repo);
    world.coding_operational_ref_ok = validator.ref_exists(&repo, &base_ref);
}

#[when(expr = "the coding runtime resolves skill {string}")]
fn when_resolve_skill(world: &mut QuectoWorld, skill: String) {
    let ws = world
        .coding_operational_workspace
        .as_ref()
        .expect("workspace should exist")
        .clone();
    let resolver = WorkspaceSkillResolver::new(ws);
    world.coding_operational_skill_ok = resolver.skill_exists(&skill);
}

#[when("coding_job wiring is applied for CLI and definitions are listed")]
fn when_cli_wiring_list_defs(world: &mut QuectoWorld) {
    let ws = world
        .coding_operational_workspace
        .as_ref()
        .expect("workspace should exist")
        .clone();
    let registry = world
        .coding_operational_registry
        .as_mut()
        .expect("registry should exist");
    let base_td = tempfile::TempDir::new().unwrap();
    let _ = build_coding_lifecycle(registry, &ws, base_td.path());
    world._extra_temp_dirs.push(base_td);
    world.coding_operational_definitions = registry.definitions();
}

#[when("coding_job wiring is applied for gateway and definitions are listed")]
fn when_gateway_wiring_list_defs(world: &mut QuectoWorld) {
    when_cli_wiring_list_defs(world);
}

#[when(expr = "I execute coding_job run for repo {string} and base ref {string}")]
fn when_run_workspace_tool(world: &mut QuectoWorld, repo: String, base_ref: String) {
    let tool = world
        .coding_operational_tool
        .as_ref()
        .expect("workspace-backed tool should be initialized");
    let input = serde_json::json!({
        "action": "run",
        "goal": "operational run",
        "repo": repo,
        "base_ref": base_ref,
    })
    .to_string();
    let result = exec_tool(tool, &input);
    if !result.is_error {
        let json: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        world.coding_operational_last_job_id = Some(json["job_id"].as_str().unwrap().to_string());
    }
    world.coding_operational_last_result = Some(result);
}

#[when("I execute coding_job status for the created job")]
fn when_status_workspace_tool(world: &mut QuectoWorld) {
    let tool = world
        .coding_operational_tool
        .as_ref()
        .expect("workspace-backed tool should be initialized");
    let job_id = world
        .coding_operational_last_job_id
        .as_ref()
        .expect("job id should exist")
        .clone();
    let input = serde_json::json!({"action":"status", "job_id": job_id}).to_string();
    world.coding_operational_last_result = Some(exec_tool(tool, &input));
}

#[when("I cancel and cleanup the created coding job")]
fn when_cancel_and_cleanup(world: &mut QuectoWorld) {
    let tool = world
        .coding_operational_tool
        .as_ref()
        .expect("workspace-backed tool should be initialized");
    let job_id = world
        .coding_operational_last_job_id
        .as_ref()
        .expect("job id should exist")
        .clone();
    let cancel_input = serde_json::json!({"action":"cancel", "job_id": job_id}).to_string();
    let cancel_result = exec_tool(tool, &cancel_input);
    assert!(
        !cancel_result.is_error,
        "cancel should succeed before cleanup: {}",
        cancel_result.content
    );
    let cleanup_input =
        serde_json::json!({"action":"cleanup", "job_id": world.coding_operational_last_job_id.as_ref().unwrap()})
            .to_string();
    world.coding_operational_last_result = Some(exec_tool(tool, &cleanup_input));
}

#[when("I list coding jobs")]
fn when_list_jobs(world: &mut QuectoWorld) {
    let tool = world
        .coding_operational_tool
        .as_ref()
        .expect("workspace-backed tool should be initialized");
    world.coding_operational_last_result = Some(exec_tool(tool, r#"{"action":"list"}"#));
}

#[when("coding coordinator scope policy is queried")]
fn when_scope_policy_queried(world: &mut QuectoWorld) {
    world.coding_operational_repo_ok =
        shared::cli_coding_coordinator_scope() == CodingCoordinatorScopePolicy::PerSession;
    world.coding_operational_ref_ok = shared::gateway_inbound_coding_coordinator_scope()
        == CodingCoordinatorScopePolicy::PerSession;
    world.coding_operational_skill_ok = shared::gateway_background_coding_coordinator_scope()
        == CodingCoordinatorScopePolicy::Shared;
}

#[then("repo validation should succeed")]
fn then_repo_ok(world: &mut QuectoWorld) {
    assert!(world.coding_operational_repo_ok);
}

#[then("base ref validation should succeed")]
fn then_ref_ok(world: &mut QuectoWorld) {
    assert!(world.coding_operational_ref_ok);
}

#[then("skill resolution should succeed")]
fn then_skill_ok(world: &mut QuectoWorld) {
    assert!(world.coding_operational_skill_ok);
}

#[then(expr = "the registry should include a tool named {string}")]
fn then_registry_includes(world: &mut QuectoWorld, name: String) {
    let has = world
        .coding_operational_definitions
        .iter()
        .any(|d| d.name == name);
    assert!(has, "expected definitions to include coding_job");
}

#[then("the coding_job tool result should not be an error")]
fn then_tool_not_error(world: &mut QuectoWorld) {
    let result = world
        .coding_operational_last_result
        .as_ref()
        .expect("tool result should exist");
    assert!(
        !result.is_error,
        "expected success, got: {}",
        result.content
    );
}

#[then(expr = "the coding_job tool result should contain {string}")]
fn then_tool_contains(world: &mut QuectoWorld, needle: String) {
    let result = world
        .coding_operational_last_result
        .as_ref()
        .expect("tool result should exist");
    assert!(
        result.content.contains(&needle),
        "expected '{}' in result: {}",
        needle,
        result.content
    );
}

#[then(expr = "CLI coding coordinator scope should be {string}")]
fn then_cli_scope(_world: &mut QuectoWorld, expected: String) {
    let got = shared::cli_coding_coordinator_scope();
    let exp = if expected == "per_session" {
        CodingCoordinatorScopePolicy::PerSession
    } else {
        CodingCoordinatorScopePolicy::Shared
    };
    assert_eq!(got, exp);
}

#[then(expr = "gateway inbound coding coordinator scope should be {string}")]
fn then_gateway_inbound_scope(_world: &mut QuectoWorld, expected: String) {
    let got = shared::gateway_inbound_coding_coordinator_scope();
    let exp = if expected == "per_session" {
        CodingCoordinatorScopePolicy::PerSession
    } else {
        CodingCoordinatorScopePolicy::Shared
    };
    assert_eq!(got, exp);
}

#[then(expr = "gateway background coding coordinator scope should be {string}")]
fn then_gateway_background_scope(_world: &mut QuectoWorld, expected: String) {
    let got = shared::gateway_background_coding_coordinator_scope();
    let exp = if expected == "per_session" {
        CodingCoordinatorScopePolicy::PerSession
    } else {
        CodingCoordinatorScopePolicy::Shared
    };
    assert_eq!(got, exp);
}

// ========================================================================
// Lifecycle-wired tool (supports create/import via DriverJobService)
// ========================================================================

#[given("a lifecycle-wired coding_job tool")]
fn given_lifecycle_wired_tool(world: &mut QuectoWorld) {
    let ws = tempfile::TempDir::new().unwrap();
    let ws_path = ws.path().to_path_buf();
    let base_td = tempfile::TempDir::new().unwrap();

    let sandbox = Sandbox::new(Some(ws_path.clone()), true);
    let mut registry = ToolRegistryImpl::with_core_tools(ws_path.clone(), sandbox);
    let _ = build_coding_lifecycle(&mut registry, &ws_path, base_td.path());

    // Verify the coding_job tool is registered
    let has_tool = registry
        .definitions()
        .iter()
        .any(|d| d.name == "coding_job");
    assert!(has_tool, "coding_job should be registered");

    world.coding_operational_workspace = Some(ws_path);
    world.coding_operational_registry = Some(registry);
    world._extra_temp_dirs.push(ws);
    world._extra_temp_dirs.push(base_td);
}

fn exec_registry_tool(
    registry: &ToolRegistryImpl,
    input: &str,
) -> quecto::domain::tool::ToolResult {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(registry.execute("coding_job", input))
        .expect("execute should not fail")
}

#[when(expr = "I execute coding_job create with name {string}")]
fn when_create_repo(world: &mut QuectoWorld, name: String) {
    let registry = world
        .coding_operational_registry
        .as_ref()
        .expect("registry should exist");
    let input = serde_json::json!({"action": "create", "name": name}).to_string();
    world.coding_operational_last_result = Some(exec_registry_tool(registry, &input));
}

#[when(expr = "I execute coding_job import with url {string}")]
fn when_import_repo_no_name(world: &mut QuectoWorld, url: String) {
    let registry = world
        .coding_operational_registry
        .as_ref()
        .expect("registry should exist");
    let input = serde_json::json!({"action": "import", "url": url}).to_string();
    world.coding_operational_last_result = Some(exec_registry_tool(registry, &input));
}

#[when(expr = "I execute coding_job import with url {string} and name {string}")]
fn when_import_repo_with_name(world: &mut QuectoWorld, url: String, name: String) {
    let registry = world
        .coding_operational_registry
        .as_ref()
        .expect("registry should exist");
    let input = serde_json::json!({"action": "import", "url": url, "name": name}).to_string();
    world.coding_operational_last_result = Some(exec_registry_tool(registry, &input));
}

#[when(expr = "I execute coding_job run for repo {string} on the lifecycle tool")]
fn when_run_on_lifecycle_tool(world: &mut QuectoWorld, repo: String) {
    let registry = world
        .coding_operational_registry
        .as_ref()
        .expect("registry should exist");
    let input = serde_json::json!({
        "action": "run",
        "goal": "build something",
        "repo": repo,
        "base_ref": "main",
    })
    .to_string();
    world.coding_operational_last_result = Some(exec_registry_tool(registry, &input));
}

#[then("the coding_job tool result should be an error")]
fn then_tool_is_error(world: &mut QuectoWorld) {
    let result = world
        .coding_operational_last_result
        .as_ref()
        .expect("tool result should exist");
    assert!(
        result.is_error,
        "expected error, got success: {}",
        result.content
    );
}
