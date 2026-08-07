use super::*;

// Slice 2 (#1369): join, list, and kill shared script-managed environments.
// ===========================================================================

pub(crate) fn shared_log_path(world: &QuectoWorld) -> PathBuf {
    let cfg_path = PathBuf::from(world.config_path.clone().unwrap());
    cfg_path.parent().unwrap().join("container-env-log.jsonl")
}

pub(crate) fn shared_invocations(world: &QuectoWorld) -> Vec<serde_json::Value> {
    let text = std::fs::read_to_string(shared_log_path(world)).unwrap_or_default();
    text.lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

pub(crate) fn write_executable(path: &PathBuf, content: String) {
    std::fs::write(path, content).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut p = std::fs::metadata(path).unwrap().permissions();
        p.set_mode(0o700);
        std::fs::set_permissions(path, p).unwrap();
    }
}

/// Configure a full create/exec/kill (plus rollback cleanup) script set that
/// records every invocation kind to a shared JSONL log, then rewrites the
/// session config's `container_configs` to point at it.
pub(crate) fn given_shared_script_spawn(world: &mut QuectoWorld, kill_fails_once: bool) {
    spawn_tool_steps::given_live_spawn_agent_cmd_mock_child(world);
    let base = base_path(world);
    let cfg_path = PathBuf::from(world.config_path.clone().unwrap());
    let cfg_dir = cfg_path.parent().unwrap().to_path_buf();
    let log = shared_log_path(world);

    let create = base.join("env-create.sh");
    write_executable(
        &create,
        format!(
            r#"#!/usr/bin/env bash
set -euo pipefail
env_id="env-$RANDOM-$$"
echo "{{\"kind\":\"create\",\"script\":\"${{QUECTO_CONTAINER_CONFIG:-}}\",\"env_ref\":\"${{QUECTO_CONTAINER_ENVIRONMENT_REF:-}}\",\"env_id\":\"$env_id\"}}" >> '{log}'
socket_path=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--" ]; then shift; break; fi
  shift
done
prev=""
for arg in "$@"; do
  if [ "$prev" = "--socket" ]; then socket_path="$arg"; break; fi
  prev="$arg"
done
"$@" >/dev/null 2>&1 &
printf '{{"environment_id":"%s","workspace_path":"%s","metadata":{{}},"socket_path":"%s"}}' "$env_id" "$PWD/workspace-$env_id" "$socket_path"
"#,
            log = log.display()
        ),
    );

    let exec = base.join("env-exec.sh");
    write_executable(
        &exec,
        format!(
            r#"#!/usr/bin/env bash
set -euo pipefail
echo "{{\"kind\":\"exec\",\"script\":\"${{QUECTO_CONTAINER_CONFIG:-}}\",\"env_id\":\"${{QUECTO_CONTAINER_ENVIRONMENT_ID:-}}\"}}" >> '{log}'
socket_path=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--" ]; then shift; break; fi
  shift
done
prev=""
for arg in "$@"; do
  if [ "$prev" = "--socket" ]; then socket_path="$arg"; break; fi
  prev="$arg"
done
"$@" >/dev/null 2>&1 &
printf '{{"socket_path":"%s","metadata":{{}}}}' "$socket_path"
"#,
            log = log.display()
        ),
    );

    // The alternate script set's exec is a DIFFERENT executable that logs a
    // fixed marker: an implementation that retains only the script *name* but
    // re-resolves the argv from the current config at join time would run this
    // file and fail the retained-script assertion.
    let exec_alt = base.join("env-exec-alt.sh");
    write_executable(
        &exec_alt,
        format!(
            r#"#!/usr/bin/env bash
set -euo pipefail
echo "{{\"kind\":\"exec\",\"script\":\"alternate-argv\",\"env_id\":\"${{QUECTO_CONTAINER_ENVIRONMENT_ID:-}}\"}}" >> '{log}'
exit 1
"#,
            log = log.display()
        ),
    );

    let kill = base.join("env-kill.sh");
    let fail_marker = cfg_dir.join("kill-failed-once.marker");
    let fail_clause = if kill_fails_once {
        format!(
            r#"if [ ! -e '{marker}' ]; then touch '{marker}'; echo "simulated kill failure" >&2; exit 1; fi"#,
            marker = fail_marker.display()
        )
    } else {
        String::new()
    };
    write_executable(
        &kill,
        format!(
            r#"#!/usr/bin/env bash
set -euo pipefail
echo "{{\"kind\":\"kill\",\"env_id\":\"${{QUECTO_CONTAINER_ENVIRONMENT_ID:-}}\"}}" >> '{log}'
{fail_clause}
"#,
            log = log.display()
        ),
    );

    let cleanup = base.join("env-cleanup.sh");
    write_executable(
        &cleanup,
        format!(
            r#"#!/usr/bin/env bash
set -euo pipefail
echo "{{\"kind\":\"cleanup\",\"env_id\":\"${{QUECTO_CONTAINER_ENVIRONMENT_ID:-}}\"}}" >> '{log}'
"#,
            log = log.display()
        ),
    );

    let mut v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&cfg_path).unwrap()).unwrap();
    let script_set = serde_json::json!({
        "create": [create.to_string_lossy()],
        "exec": [exec.to_string_lossy()],
        "kill": [kill.to_string_lossy()],
        "cleanup": [cleanup.to_string_lossy()],
    });
    let mut alternate_set = script_set.clone();
    alternate_set["exec"] = serde_json::json!([exec_alt.to_string_lossy()]);
    let mut default_set = script_set.clone();
    default_set["default"] = serde_json::json!(true);
    v["container_configs"] = serde_json::json!({
        "default": default_set, "alternate": alternate_set
    });
    // No git fixture: configs own their source (#1410) and the parent's
    // location/checkout is irrelevant to container semantics.
    std::fs::write(&cfg_path, serde_json::to_string_pretty(&v).unwrap()).unwrap();

    // Rebuild the SpawnTool with notification + live-event channels wired
    // (mirroring composition), so scenarios can observe passive exit notes and
    // state-changed events for script-managed children (#1369 slice 3).
    let subagent_registry_for_spawn = world
        .agent_cmd_registry
        .as_ref()
        .expect("agent_cmd registry")
        .clone();
    let (notify_tx, notify_rx) =
        quecto::infrastructure::tools::subagent_registry::new_notification_channel();
    let (broadcast_tx, broadcast_rx) = tokio::sync::broadcast::channel::<String>(64);
    world.spawn_tool = Some(
        SpawnTool::with_base_dir(vec![], true, base.clone())
            .with_socket_dir(base.join("sockets"))
            .with_registry(subagent_registry_for_spawn)
            .with_notify_tx(notify_tx)
            .with_event_forwarding(Some(broadcast_tx), None),
    );
    world.notify_rx = Some(notify_rx);
    world.spawn_broadcast_rx = Some(broadcast_rx);

    // Rebuild the AgentCmdTool with environment control sharing the
    // SpawnTool's session environment registry, mirroring composition wiring.
    let environment_registry = world
        .spawn_tool
        .as_ref()
        .expect("spawn tool")
        .environment_registry()
        .clone();
    let subagent_registry = world
        .agent_cmd_registry
        .as_ref()
        .expect("agent_cmd registry")
        .clone();
    let kill_port = std::sync::Arc::new(
        quecto::infrastructure::tools::environment_kill::ScriptEnvironmentKill::new(
            subagent_registry.clone(),
            None,
        ),
    );
    let environment_control = std::sync::Arc::new(
        quecto::application::environment_control::EnvironmentControlUseCase::new(
            environment_registry,
            kill_port,
        ),
    );
    world.agent_cmd_tool = Some(
        quecto::infrastructure::tools::agent_cmd::AgentCmdTool::new(subagent_registry)
            .with_environment_control(environment_control),
    );
}

/// Scenario-scoped runtime shared by every environment step, so monitor and
/// proxy-bridge tasks spawned during launch stay alive across steps.
pub(crate) fn env_runtime(world: &mut QuectoWorld) -> std::sync::Arc<tokio::runtime::Runtime> {
    world
        .env_rt
        .get_or_insert_with(|| std::sync::Arc::new(tokio::runtime::Runtime::new().unwrap()))
        .clone()
}

pub(crate) fn execute_env_spawn(
    world: &mut QuectoWorld,
    agent_id: &str,
    mut args: serde_json::Value,
) {
    args["config"] = serde_json::json!(world.config_path.clone().unwrap());
    let rt = env_runtime(world);
    let tool = world.spawn_tool.as_ref().expect("spawn tool");
    let result = match rt.block_on(tool.execute(&args.to_string())) {
        Ok(r) => r,
        Err(e) => ToolResult {
            content: e.to_string(),
            is_error: true,
            image_blocks: vec![],
        },
    };
    if !result.is_error {
        if let Some(uuid) = result
            .content
            .split("uuid=")
            .nth(1)
            .and_then(|s| s.split(')').next())
        {
            world
                .agent_spawn_uuids
                .insert(agent_id.to_string(), uuid.to_string());
        }
        if let Some(env_ref) = result
            .content
            .split("environment_ref=")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
        {
            world.agent_env_refs.insert(
                agent_id.to_string(),
                env_ref.trim_end_matches(')').to_string(),
            );
        }
        if let Some(workspace) = result
            .content
            .split("workspace=")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
        {
            world.agent_workspaces.insert(
                agent_id.to_string(),
                workspace.trim_end_matches(['.', ')']).to_string(),
            );
        }
    }
    world.spawn_result = Some(result);
}

pub(crate) fn run_container_command(
    world: &mut QuectoWorld,
    args: serde_json::Value,
) -> ToolResult {
    let rt = env_runtime(world);
    let tool = world.agent_cmd_tool.as_ref().expect("agent_cmd tool");
    match rt.block_on(tool.execute(&args.to_string())) {
        Ok(r) => r,
        Err(e) => ToolResult {
            content: e.to_string(),
            is_error: true,
            image_blocks: vec![],
        },
    }
}

/// Fetch the authoritative listing and return the entry for `env_ref`.
pub(crate) fn container_listing_entry(world: &mut QuectoWorld, env_ref: &str) -> serde_json::Value {
    let result = run_container_command(
        world,
        serde_json::json!({"agent_id": "*", "command": "get_containers"}),
    );
    assert!(
        !result.is_error,
        "get_containers failed: {}",
        result.content
    );
    let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap_or_else(|e| {
        panic!(
            "get_containers should return JSON: {e}; got {}",
            result.content
        )
    });
    let containers = parsed["containers"]
        .as_array()
        .unwrap_or_else(|| panic!("get_containers should list containers: {parsed}"));
    containers
        .iter()
        .find(|c| c["ref"].as_str() == Some(env_ref))
        .cloned()
        .unwrap_or_else(|| panic!("expected listing to include {env_ref}: {containers:?}"))
}

// --- Given ---

#[given("shared script-managed subagent spawning is available")]
fn given_shared_spawn(world: &mut QuectoWorld) {
    given_shared_script_spawn(world, false);
}

#[given("shared script-managed subagent spawning is available with a kill script that fails once")]
fn given_shared_spawn_kill_fails_once(world: &mut QuectoWorld) {
    given_shared_script_spawn(world, true);
}

#[given(
    expr = "script-managed child {string} is running in a shared environment with task {string}"
)]
fn given_shared_child_running(world: &mut QuectoWorld, agent_id: String, task: String) {
    execute_env_spawn(
        world,
        &agent_id,
        serde_json::json!({"agent_id":agent_id,"task":task,"container":true,"read_only":true}),
    );
    let r = world.spawn_result.as_ref().unwrap();
    assert!(!r.is_error, "shared spawn failed: {}", r.content);
}

#[given(
    expr = "script-managed child {string} is running in a shared environment named {string} with task {string}"
)]
fn given_shared_child_running_named(
    world: &mut QuectoWorld,
    agent_id: String,
    name: String,
    task: String,
) {
    execute_env_spawn(
        world,
        &agent_id,
        serde_json::json!({"agent_id":agent_id,"task":task,"container":{"mode":"new","name":name},"read_only":true}),
    );
    let r = world.spawn_result.as_ref().unwrap();
    assert!(!r.is_error, "named shared spawn failed: {}", r.content);
}

#[given(
    expr = "read-only subagent {string} has joined existing environment ref {string} with task {string}"
)]
fn given_joined_existing(world: &mut QuectoWorld, agent_id: String, env_ref: String, task: String) {
    when_join_existing_ref(world, agent_id, env_ref, task);
    let r = world.spawn_result.as_ref().unwrap();
    assert!(!r.is_error, "join failed: {}", r.content);
}

#[given(expr = "the configured default container script changes to {string}")]
fn given_default_script_changes(world: &mut QuectoWorld, new_default: String) {
    let cfg_path = PathBuf::from(world.config_path.clone().unwrap());
    let mut v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&cfg_path).unwrap()).unwrap();
    // Move the `"default": true` label to the new entry (#1410); the
    // retained-script assertion must be able to distinguish the two sets.
    let names: Vec<String> = v["container_configs"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    for name in names {
        v["container_configs"][&name]["default"] = serde_json::json!(name == new_default);
    }
    std::fs::write(&cfg_path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
}

// --- When ---

#[when(
    expr = "I spawn read-only subagent {string} into existing environment ref {string} with task {string}"
)]
fn when_join_existing_ref(
    world: &mut QuectoWorld,
    agent_id: String,
    env_ref: String,
    task: String,
) {
    execute_env_spawn(
        world,
        &agent_id,
        serde_json::json!({"agent_id":agent_id,"task":task,"container":{"mode":"existing","ref":env_ref},"read_only":true}),
    );
}

#[when(
    expr = "I spawn read-only subagent {string} into existing environment name {string} with task {string}"
)]
fn when_join_existing_name(world: &mut QuectoWorld, agent_id: String, name: String, task: String) {
    execute_env_spawn(
        world,
        &agent_id,
        serde_json::json!({"agent_id":agent_id,"task":task,"container":{"mode":"existing","name":name},"read_only":true}),
    );
}

#[when(
    expr = "I spawn script-managed subagent {string} into a new shared environment with task {string}"
)]
fn when_spawn_new_shared(world: &mut QuectoWorld, agent_id: String, task: String) {
    execute_env_spawn(
        world,
        &agent_id,
        serde_json::json!({"agent_id":agent_id,"task":task,"container":true,"read_only":true}),
    );
}

#[when(expr = "I run container command {string}")]
fn when_run_container_command(world: &mut QuectoWorld, command: String) {
    let result = run_container_command(
        world,
        serde_json::json!({"agent_id": "*", "command": command}),
    );
    world.container_cmd_result = Some(result);
}

#[when(expr = "I kill container {string}")]
fn when_kill_container(world: &mut QuectoWorld, env_ref: String) {
    let result = run_container_command(
        world,
        serde_json::json!({"agent_id": "*", "command": "kill_container", "ref": env_ref}),
    );
    world.container_cmd_result = Some(result);
}

#[when(expr = "I kill subagent {string}")]
fn when_kill_subagent(world: &mut QuectoWorld, agent_id: String) {
    // Action only: success/failure is asserted by Then steps (or by the
    // Given wrapper below when the kill is scenario context).
    let result = run_container_command(
        world,
        serde_json::json!({"agent_id": agent_id, "command": "kill"}),
    );
    world.agent_cmd_result = Some(result);
}

// --- Context wrappers (kills as preconditions of later behaviour) ---

#[given(expr = "environment {string} has been killed")]
fn given_environment_killed(world: &mut QuectoWorld, env_ref: String) {
    when_kill_container(world, env_ref.clone());
    let r = world.container_cmd_result.as_ref().unwrap();
    assert!(
        !r.is_error,
        "precondition kill of {env_ref} failed: {}",
        r.content
    );
}

#[given(expr = "the first kill of environment {string} has failed")]
fn given_first_kill_failed(world: &mut QuectoWorld, env_ref: String) {
    when_kill_container(world, env_ref.clone());
    let r = world.container_cmd_result.as_ref().unwrap();
    assert!(
        r.is_error,
        "precondition: first kill of {env_ref} must fail: {}",
        r.content
    );
}

#[given(expr = "subagent {string} has been killed")]
fn given_subagent_killed(world: &mut QuectoWorld, agent_id: String) {
    when_kill_subagent(world, agent_id.clone());
    let r = world.agent_cmd_result.as_ref().unwrap();
    assert!(
        !r.is_error,
        "precondition kill of {agent_id} failed: {}",
        r.content
    );
}

// --- Then ---

#[then(
    expr = "the script-managed runtime should have joined an existing environment exactly {int} time(s)"
)]
fn then_join_count(world: &mut QuectoWorld, n: i32) {
    let inv = shared_invocations(world);
    let count = inv.iter().filter(|v| v["kind"] == "exec").count() as i32;
    assert_eq!(count, n, "invocations: {inv:?}");
}

#[then(expr = "the script-managed runtime should have created exactly {int} environment(s)")]
fn then_create_count(world: &mut QuectoWorld, n: i32) {
    let inv = shared_invocations(world);
    let count = inv.iter().filter(|v| v["kind"] == "create").count() as i32;
    assert_eq!(count, n, "invocations: {inv:?}");
}

#[then(expr = "the script-managed runtime should have killed an environment exactly {int} time(s)")]
fn then_kill_count(world: &mut QuectoWorld, n: i32) {
    let inv = shared_invocations(world);
    let kills: Vec<&serde_json::Value> = inv.iter().filter(|v| v["kind"] == "kill").collect();
    assert_eq!(kills.len() as i32, n, "invocations: {inv:?}");
    // Every kill must target an environment id this session actually created:
    // killing the wrong id (or an empty one) is not cleanup.
    let created_ids: Vec<&str> = inv
        .iter()
        .filter(|v| v["kind"] == "create")
        .filter_map(|v| v["env_id"].as_str())
        .collect();
    for kill in kills {
        let env_id = kill["env_id"].as_str().unwrap_or_default();
        assert!(
            !env_id.is_empty() && created_ids.contains(&env_id),
            "kill must target a created environment id: kill={kill} created={created_ids:?}"
        );
    }
}

#[then(expr = "the join should have used the retained {string} script set")]
fn then_exec_retained_script(world: &mut QuectoWorld, script: String) {
    let inv = shared_invocations(world);
    let exec = inv
        .iter()
        .find(|v| v["kind"] == "exec")
        .unwrap_or_else(|| panic!("no exec invocation recorded: {inv:?}"));
    assert_eq!(
        exec["script"].as_str(),
        Some(script.as_str()),
        "exec must use the environment's retained script set, not the current default: {inv:?}"
    );
}

#[then(expr = "subagents {string} and {string} should report different agent UUIDs")]
fn then_different_agent_uuids(world: &mut QuectoWorld, a: String, b: String) {
    let ua = world
        .agent_spawn_uuids
        .get(&a)
        .unwrap_or_else(|| panic!("no captured uuid for {a}: {:?}", world.agent_spawn_uuids))
        .clone();
    let ub = world
        .agent_spawn_uuids
        .get(&b)
        .unwrap_or_else(|| panic!("no captured uuid for {b}: {:?}", world.agent_spawn_uuids))
        .clone();
    assert_ne!(ua, ub, "agents must have distinct agent UUIDs");
}

#[then(expr = "subagents {string} and {string} should share environment reference {string}")]
fn then_shared_env_ref(world: &mut QuectoWorld, a: String, b: String, env_ref: String) {
    let ra = world.agent_env_refs.get(&a).cloned();
    let rb = world.agent_env_refs.get(&b).cloned();
    assert_eq!(ra.as_deref(), Some(env_ref.as_str()), "{a} env ref: {ra:?}");
    assert_eq!(rb.as_deref(), Some(env_ref.as_str()), "{b} env ref: {rb:?}");
}

#[then(expr = "subagents {string} and {string} should share the same workspace")]
fn then_shared_workspace(world: &mut QuectoWorld, a: String, b: String) {
    // Each agent's workspace comes from its OWN spawn result; the joiner must
    // report the creator's workspace, and the listing must agree.
    let wa = world
        .agent_workspaces
        .get(&a)
        .unwrap_or_else(|| {
            panic!(
                "no captured workspace for {a}: {:?}",
                world.agent_workspaces
            )
        })
        .clone();
    let wb = world
        .agent_workspaces
        .get(&b)
        .unwrap_or_else(|| {
            panic!(
                "no captured workspace for {b}: {:?}",
                world.agent_workspaces
            )
        })
        .clone();
    assert!(!wa.is_empty(), "workspace for {a} must be reported");
    assert_eq!(wa, wb, "{a} and {b} must share one workspace");
    let env_ref = world
        .agent_env_refs
        .get(&a)
        .cloned()
        .expect("captured env ref");
    let entry = container_listing_entry(world, &env_ref);
    assert_eq!(
        entry["workspace"].as_str(),
        Some(wa.as_str()),
        "listing workspace must match the members' reported workspace: {entry}"
    );
}

#[then(expr = "subagents {string} and {string} should both be listed as members of {string}")]
fn then_both_listed_as_members(world: &mut QuectoWorld, a: String, b: String, env_ref: String) {
    let entry = container_listing_entry(world, &env_ref);
    let members: Vec<&str> = entry["members"]
        .as_array()
        .unwrap_or_else(|| panic!("listing must report members: {entry}"))
        .iter()
        .filter_map(|m| m.as_str())
        .collect();
    let ua = world.agent_spawn_uuids.get(&a).cloned().unwrap_or_default();
    let ub = world.agent_spawn_uuids.get(&b).cloned().unwrap_or_default();
    assert!(
        !ua.is_empty()
            && !ub.is_empty()
            && members.contains(&ua.as_str())
            && members.contains(&ub.as_str()),
        "both agent UUIDs must be members of {env_ref}: members={members:?} a={ua} b={ub}"
    );
}

#[then(expr = "the spawn result should fail because environment {string} is unknown")]
fn then_env_unknown(world: &mut QuectoWorld, env_ref: String) {
    let r = world.spawn_result.as_ref().unwrap();
    assert!(
        r.is_error && r.content.contains(&env_ref) && r.content.contains("unknown"),
        "expected unknown-ref failure for {env_ref}: {}",
        r.content
    );
}

#[then(expr = "the spawn result should fail because environment name {string} is ambiguous")]
fn then_env_ambiguous(world: &mut QuectoWorld, name: String) {
    let r = world.spawn_result.as_ref().unwrap();
    assert!(
        r.is_error && r.content.contains(&name) && r.content.contains("ambiguous"),
        "expected ambiguous-name failure for {name}: {}",
        r.content
    );
}

#[then(expr = "the spawn result should fail because environment {string} is stopped")]
fn then_env_stopped(world: &mut QuectoWorld, env_ref: String) {
    let r = world.spawn_result.as_ref().unwrap();
    assert!(
        r.is_error && r.content.contains(&env_ref) && r.content.contains("stopped"),
        "expected stopped-ref failure for {env_ref}: {}",
        r.content
    );
}

#[then("the container command result should not be an error")]
fn then_container_cmd_ok(world: &mut QuectoWorld) {
    let r = world
        .container_cmd_result
        .as_ref()
        .expect("no container command result");
    assert!(!r.is_error, "container command failed: {}", r.content);
}

#[then(expr = "the container command result should be an error mentioning {string}")]
fn then_container_cmd_error_mentioning(world: &mut QuectoWorld, expected: String) {
    let r = world
        .container_cmd_result
        .as_ref()
        .expect("no container command result");
    assert!(
        r.is_error && r.content.contains(&expected),
        "expected error mentioning '{expected}': {}",
        r.content
    );
}

#[then(
    expr = "the container listing should include {string} with status {string} and {int} member(s)"
)]
fn then_listing_status_members(world: &mut QuectoWorld, env_ref: String, status: String, n: i32) {
    let entry = container_listing_entry(world, &env_ref);
    assert_eq!(
        entry["status"].as_str(),
        Some(status.as_str()),
        "listing entry: {entry}"
    );
    let members = entry["members"].as_array().cloned().unwrap_or_default();
    assert_eq!(members.len() as i32, n, "listing entry: {entry}");
}

#[then(
    expr = "the container listing should include {string} with status {string} and a last error"
)]
fn then_listing_status_last_error(world: &mut QuectoWorld, env_ref: String, status: String) {
    let entry = container_listing_entry(world, &env_ref);
    assert_eq!(
        entry["status"].as_str(),
        Some(status.as_str()),
        "listing entry: {entry}"
    );
    let last_error = entry["last_error"].as_str().unwrap_or_default();
    assert!(
        !last_error.is_empty(),
        "cleanup failure must persist an actionable last error: {entry}"
    );
}

// --- Slice 2 "empty" state: a committed environment before its first member ---

#[when(expr = "I start spawning subagent {string} into a gated new environment")]
fn when_start_gated_spawn(world: &mut QuectoWorld, agent_id: String) {
    let base = base_path(world);
    let cfg_path = PathBuf::from(world.config_path.clone().unwrap());
    let log = shared_log_path(world);
    let gate = cfg_path.parent().unwrap().join("child-gate.marker");
    let _ = std::fs::remove_file(&gate);

    // Same create contract as the shared fixture, but the child only starts
    // (and its socket only opens) once the gate file exists — holding the
    // environment in its committed-but-memberless window.
    let create = base.join("env-create-gated.sh");
    write_executable(
        &create,
        format!(
            r#"#!/usr/bin/env bash
set -euo pipefail
env_id="env-gated-$$"
echo "{{\"kind\":\"create\",\"script\":\"${{QUECTO_CONTAINER_CONFIG:-}}\",\"env_ref\":\"${{QUECTO_CONTAINER_ENVIRONMENT_REF:-}}\",\"env_id\":\"$env_id\"}}" >> '{log}'
socket_path=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--" ]; then shift; break; fi
  shift
done
prev=""
for arg in "$@"; do
  if [ "$prev" = "--socket" ]; then socket_path="$arg"; break; fi
  prev="$arg"
done
( while [ ! -e '{gate}' ]; do sleep 0.05; done; "$@" ) >/dev/null 2>&1 &
printf '{{"environment_id":"%s","workspace_path":"%s","metadata":{{}},"socket_path":"%s"}}' "$env_id" "$PWD/workspace-$env_id" "$socket_path"
"#,
            log = log.display(),
            gate = gate.display(),
        ),
    );
    let mut v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&cfg_path).unwrap()).unwrap();
    v["container_configs"]["default"]["create"] = serde_json::json!([create.to_string_lossy()]);
    std::fs::write(&cfg_path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
    world.gate_path = Some(gate);

    let tool = world.spawn_tool.take().expect("spawn tool");
    let args = serde_json::json!({
        "agent_id": agent_id,
        "task": "GATED_EMPTY_MARKER",
        "container": true,
        "read_only": true,
        "config": world.config_path.clone().unwrap(),
    });
    world.gated_spawn = Some(std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = match rt.block_on(tool.execute(&args.to_string())) {
            Ok(r) => r,
            Err(e) => ToolResult {
                content: e.to_string(),
                is_error: true,
                image_blocks: vec![],
            },
        };
        (tool, result)
    }));
}

#[then(
    expr = "the container listing should eventually include {string} with status {string} and {int} members"
)]
fn then_listing_eventually(world: &mut QuectoWorld, env_ref: String, status: String, n: i32) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    loop {
        let result = run_container_command(
            world,
            serde_json::json!({"agent_id": "*", "command": "get_containers"}),
        );
        if !result.is_error {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&result.content) {
                if let Some(entry) = parsed["containers"]
                    .as_array()
                    .and_then(|cs| cs.iter().find(|c| c["ref"].as_str() == Some(&env_ref)))
                {
                    if entry["status"].as_str() == Some(&status) {
                        assert_eq!(
                            entry["members"].as_array().map(|m| m.len()),
                            Some(n as usize),
                            "entry: {entry}"
                        );
                        return;
                    }
                }
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "environment {env_ref} never reached status {status}: {:?}",
            result.content
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

#[when("I release the gated environment child")]
fn when_release_gate(world: &mut QuectoWorld) {
    std::fs::write(world.gate_path.as_ref().expect("gate path"), b"go").unwrap();
}

#[when(expr = "the gated spawn for {string} completes successfully")]
fn when_gated_spawn_completes(world: &mut QuectoWorld, agent_id: String) {
    let (tool, result) = world
        .gated_spawn
        .take()
        .expect("gated spawn in flight")
        .join()
        .expect("gated spawn thread");
    world.spawn_tool = Some(tool);
    assert!(!result.is_error, "gated spawn failed: {}", result.content);
    if let Some(env_ref) = result
        .content
        .split("environment_ref=")
        .nth(1)
        .and_then(|s| s.split_whitespace().next())
    {
        world
            .agent_env_refs
            .insert(agent_id, env_ref.trim_end_matches(')').to_string());
    }
    world.spawn_result = Some(result);
}
