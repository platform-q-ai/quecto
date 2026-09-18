//! Environments outlive sessions (#2024 S4d): the durable registry under
//! the base directory, restored (and judged against the runtime) by the
//! next harness, reachable through `get_containers` / existing-mode joins
//! / `kill_container`, and `quecto container ls|kill|gc`.
//!
//! The rig is the real `SpawnTool` and `AgentCmdTool` composed over the
//! durable registry composition builds for a base directory, with a fake
//! script set that keeps its state the way the shipped scripts do —
//! `<state>/<env_id>/{container,ref,workspace}` — and a fake "runtime": a
//! directory with one file per container holding `running` or `exited`.
//! `inspect --list` reads that directory, `kill`/`cleanup` remove the file
//! and the state dir. A restart rebuilds the tools over a freshly restored
//! registry; the real-process scenario restarts the actual binary.
use super::*;

use crate::spawn_env_steps::{container_listing_entry, write_executable};
use std::sync::Arc;
use std::time::Duration;

fn state_dir(world: &QuectoWorld) -> PathBuf {
    base_path(world).join("state")
}

fn runtime_dir(world: &QuectoWorld) -> PathBuf {
    base_path(world).join("fake-runtime")
}

fn log_path(world: &QuectoWorld) -> PathBuf {
    base_path(world).join("persist-log.jsonl")
}

fn invocations(world: &QuectoWorld, kind: &str) -> usize {
    std::fs::read_to_string(log_path(world))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|entry| entry["kind"] == kind)
        .count()
}

/// The durable registry document, as the store keeps it.
fn registry_document(world: &QuectoWorld) -> serde_json::Value {
    let path = base_path(world).join("environments.json");
    serde_json::from_str(
        &std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("durable registry {} should exist: {e}", path.display())),
    )
    .expect("durable registry is JSON")
}

fn environment_id_of(world: &QuectoWorld, env_ref: &str) -> String {
    registry_document(world)["environments"][env_ref]["environment_id"]
        .as_str()
        .unwrap_or_else(|| panic!("registry should record {env_ref}"))
        .to_string()
}

/// Write the fake script set and point the session config at it.
fn install_scripts(world: &mut QuectoWorld) {
    let base = base_path(world);
    let state = state_dir(world);
    let runtime = runtime_dir(world);
    std::fs::create_dir_all(&state).unwrap();
    std::fs::create_dir_all(&runtime).unwrap();
    let cfg_path = PathBuf::from(world.config_path.clone().unwrap());
    let cfg_dir = cfg_path.parent().unwrap().to_path_buf();
    std::fs::write(
        cfg_dir.join("fixture-processes.py"),
        include_str!("fixture_processes.py"),
    )
    .unwrap();
    let pid_dir = cfg_dir.join("env-pids");
    std::fs::create_dir_all(&pid_dir).unwrap();
    let log = log_path(world);
    let common = format!(
        r#"#!/usr/bin/env bash
set -euo pipefail
state='{state}'
runtime='{runtime}'
log='{log}'
track() {{ python3 '{pid_dir}/../fixture-processes.py' track '{pid_dir}' "$1" "$2"; }}
child_socket() {{
  while [ "$#" -gt 0 ]; do
    if [ "$1" = "--" ]; then shift; break; fi
    shift
  done
  prev=""
  for arg in "$@"; do
    if [ "$prev" = "--socket" ]; then printf '%s' "$arg"; return; fi
    prev="$arg"
  done
}}
child_command() {{
  while [ "$#" -gt 0 ]; do
    if [ "$1" = "--" ]; then shift; break; fi
    shift
  done
  printf '%s\n' "$@"
}}
"#,
        state = state.display(),
        runtime = runtime.display(),
        log = log.display(),
        pid_dir = pid_dir.display()
    );
    let create = base.join("persist-create.sh");
    write_executable(
        &create,
        format!(
            r#"{common}
env_id="env-$(date +%s%N)"
env_dir="$state/$env_id"
mkdir -p "$env_dir/workspace"
printf '%s\n' "quecto-$env_id" >"$env_dir/container"
printf '%s\n' "${{QUECTO_CONTAINER_ENVIRONMENT_REF:-}}" >"$env_dir/ref"
printf 'running\n' >"$runtime/quecto-$env_id"
echo "{{\"kind\":\"create\",\"env_id\":\"$env_id\",\"env_ref\":\"${{QUECTO_CONTAINER_ENVIRONMENT_REF:-}}\"}}" >>"$log"
socket_path="$(child_socket "$@")"
mapfile -t cmd < <(child_command "$@")
"${{cmd[@]}}" >/dev/null 2>&1 &
track "$env_id" "$!"
printf '{{"environment_id":"%s","workspace_path":"%s","metadata":{{"container":"quecto-%s"}},"socket_path":"%s"}}' "$env_id" "$env_dir/workspace" "$env_id" "$socket_path"
"#
        ),
    );
    let exec = base.join("persist-exec.sh");
    write_executable(
        &exec,
        format!(
            r#"{common}
id="${{QUECTO_CONTAINER_ENVIRONMENT_ID:?}}"
echo "{{\"kind\":\"exec\",\"env_id\":\"$id\"}}" >>"$log"
[ "$(cat "$runtime/quecto-$id" 2>/dev/null)" = running ] || {{ echo "container quecto-$id is not running" >&2; exit 1; }}
socket_path="$(child_socket "$@")"
mapfile -t cmd < <(child_command "$@")
"${{cmd[@]}}" >/dev/null 2>&1 &
track "$id" "$!"
printf '{{"socket_path":"%s","metadata":{{}}}}' "$socket_path"
"#
        ),
    );
    let inspect = base.join("persist-inspect.sh");
    write_executable(
        &inspect,
        format!(
            r#"{common}
if [ "${{1:-}}" = --list ]; then
  for f in "$runtime"/*; do
    [ -e "$f" ] || continue
    name="$(basename "$f")"
    if [ "$(cat "$f")" = running ]; then status=running; else status=dead; fi
    printf '{{"environment_id":"%s","container":"%s","status":"%s"}}\n' "${{name#quecto-}}" "$name" "$status"
  done
  exit 0
fi
id="${{QUECTO_CONTAINER_ENVIRONMENT_ID:?}}"
if [ "$(cat "$runtime/quecto-$id" 2>/dev/null)" = running ]; then
  printf '{{"status":"running","metadata":{{}}}}'
else
  printf '{{"status":"dead","metadata":{{"cause":"container-removed"}}}}'
fi
"#
        ),
    );
    let kill = base.join("persist-kill.sh");
    write_executable(
        &kill,
        format!(
            r#"{common}
op="${{1:-kill}}"
id="${{QUECTO_CONTAINER_ENVIRONMENT_ID:?}}"
echo "{{\"kind\":\"$op\",\"env_id\":\"$id\"}}" >>"$log"
python3 '{pid_dir}/../fixture-processes.py' clean '{pid_dir}' "$id" || true
rm -f "$runtime/quecto-$id"
rm -rf "$state/$id"
"#,
            pid_dir = pid_dir.display()
        ),
    );
    let mut v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&cfg_path).unwrap()).unwrap();
    v["container_configs"] = serde_json::json!({
        "default": {
            "default": true,
            "create": [create.to_string_lossy(), "--state-dir", state.to_string_lossy()],
            "exec": [exec.to_string_lossy()],
            "kill": [kill.to_string_lossy(), "kill"],
            "cleanup": [kill.to_string_lossy(), "cleanup"],
            "inspect": [inspect.to_string_lossy()],
        }
    });
    std::fs::write(&cfg_path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
}

/// (Re)build the spawn and agent_cmd tools over the durable registry
/// composition restores for `session` — what a harness start does.
fn compose_session(world: &mut QuectoWorld, session: &str) {
    let base = base_path(world);
    let registry =
        quecto::composition::environments::build_environment_registry(&base, session, true);
    let subagent_registry = world
        .agent_cmd_registry
        .as_ref()
        .expect("agent_cmd registry")
        .clone();
    let (notify_tx, notify_rx) =
        quecto::infrastructure::tools::subagent_registry::new_notification_channel();
    let (broadcast_tx, broadcast_rx) = tokio::sync::broadcast::channel::<String>(64);
    world.spawn_tool = Some(quecto::composition::subagent_lifecycle::compose_launcher(
        SpawnTool::with_base_dir(vec![], base.clone())
            .with_socket_dir(base.join("sockets"))
            .with_registry(subagent_registry.clone())
            .with_environment_registry(registry.clone())
            .with_notify_tx(notify_tx)
            .with_event_forwarding(Some(broadcast_tx.clone()), None),
    ));
    world.notify_rx = Some(notify_rx);
    world.spawn_broadcast_rx = Some(broadcast_rx);
    let owners =
        crate::agent_cmd_tool_steps::termination_owners(&subagent_registry, Some(broadcast_tx));
    let environment_control = quecto::composition::environments::build_environment_control(
        registry,
        owners.member_shutdown.clone(),
    );
    world.agent_cmd_tool = Some(
        quecto::infrastructure::tools::agent_cmd::AgentCmdTool::new(subagent_registry)
            .with_kill_tool(owners.kill_tool)
            .with_environment_control(environment_control),
    );
    world.persist_session = Some(session.to_string());
}

#[given(expr = "persistent script-managed subagent spawning is available as session {string}")]
fn given_persistent_spawn(world: &mut QuectoWorld, session: String) {
    spawn_tool_steps::given_live_spawn_agent_cmd_mock_child(world);
    install_scripts(world);
    // The CLI commands run in the same base dir and resolve the container
    // config from it: the working directory is the base's workspace, the
    // global file is the session config.
    world.cli_context.configuration =
        Some(quecto::composition::configuration::build_configuration_handles);
    world.cli_context.container_inventory =
        Some(quecto::composition::environments::build_container_inventory);
    compose_session(world, &session);
}

#[when(expr = "the harness is restarted as session {string}")]
fn when_restarted(world: &mut QuectoWorld, session: String) {
    // The old harness is gone with its monitors: dropping the scenario
    // runtime ends the tasks that watched the first session's members, so
    // nothing of session one reacts to what session two does.
    if let Some(runtime) = world.env_rt.take() {
        let runtime = Arc::try_unwrap(runtime).expect("no step holds the scenario runtime");
        runtime.shutdown_timeout(Duration::from_secs(5));
    }
    compose_session(world, &session);
}

#[given(expr = "the fake runtime loses the container of {string} behind the harness's back")]
fn given_runtime_loses_container(world: &mut QuectoWorld, env_ref: String) {
    let id = environment_id_of(world, &env_ref);
    std::fs::write(runtime_dir(world).join(format!("quecto-{id}")), "exited\n").unwrap();
}

#[given(
    expr = "the durable environment registry also records a stopped environment {string} named {string}"
)]
fn given_stopped_record(world: &mut QuectoWorld, env_ref: String, name: String) {
    let store =
        quecto::composition::environments::build_environment_registry_store(&base_path(world));
    let number: u64 = env_ref[1..].parse().unwrap();
    let id = format!("env-stopped-{number}");
    store
        .record(&quecto::domain::environment_registry::EnvironmentRecord {
            environment_ref: env_ref,
            environment_id: id.clone(),
            environment_uuid: format!("uuid-{number}"),
            name: Some(name),
            workspace_path: state_dir(world).join(&id).join("workspace"),
            repository: String::new(),
            script_name: "default".into(),
            retained_exec_argv: vec![],
            retained_kill_argv: vec![],
            retained_cleanup_argv: vec![],
            retained_inspect_argv: vec![],
            members: vec![],
            status: quecto::domain::environment_registry::EnvironmentStatus::Stopped,
            metadata: serde_json::json!({}),
            last_error: None,
            origin: quecto::domain::environment_registry::EnvironmentOrigin::Created,
            created_by: "elsewhere".into(),
            created_at: Some(0),
        })
        .unwrap();
}

#[given(
    expr = "an orphaned environment state dir {string} with an exited fake container is planted in the state dir"
)]
fn given_orphan_with_exited_container(world: &mut QuectoWorld, id: String) {
    let dir = state_dir(world).join(&id);
    std::fs::create_dir_all(dir.join("workspace")).unwrap();
    std::fs::write(dir.join("container"), format!("quecto-{id}\n")).unwrap();
    std::fs::write(runtime_dir(world).join(format!("quecto-{id}")), "exited\n").unwrap();
}

#[given(
    expr = "an orphaned environment state dir {string} without any container is planted in the state dir"
)]
fn given_orphan_without_container(world: &mut QuectoWorld, id: String) {
    let dir = state_dir(world).join(&id);
    std::fs::create_dir_all(dir.join("workspace")).unwrap();
    std::fs::write(dir.join("container"), format!("quecto-{id}\n")).unwrap();
}

#[given(expr = "an exited fake container {string} with no state dir is left in the fake runtime")]
fn given_ghost_container(world: &mut QuectoWorld, container: String) {
    std::fs::write(runtime_dir(world).join(container), "exited\n").unwrap();
}

#[then(
    expr = "the durable environment registry should record {string} with status {string} created by {string}"
)]
fn then_registry_records(
    world: &mut QuectoWorld,
    env_ref: String,
    status: String,
    created_by: String,
) {
    let document = registry_document(world);
    let entry = &document["environments"][&env_ref];
    assert!(
        !entry.is_null(),
        "registry should record {env_ref}: {document}"
    );
    assert_eq!(entry["status"].as_str(), Some(status.as_str()), "{entry}");
    assert_eq!(
        entry["created_by"].as_str(),
        Some(created_by.as_str()),
        "{entry}"
    );
}

#[then(expr = "the container listing should mark {string} as restored from session {string}")]
fn then_listing_restored(world: &mut QuectoWorld, env_ref: String, session: String) {
    let entry = container_listing_entry(world, &env_ref);
    assert_eq!(entry["restored"], serde_json::json!(true), "{entry}");
    assert_eq!(entry["session"].as_str(), Some(session.as_str()), "{entry}");
}

#[then(expr = "the container listing should carry name {string} for {string}")]
fn then_listing_name(world: &mut QuectoWorld, name: String, env_ref: String) {
    let entry = container_listing_entry(world, &env_ref);
    assert_eq!(entry["name"].as_str(), Some(name.as_str()), "{entry}");
}

#[then(
    expr = "the persistent runtime should have joined an existing environment exactly {int} time(s)"
)]
fn then_joined_times(world: &mut QuectoWorld, n: usize) {
    assert_eq!(
        invocations(world, "exec"),
        n,
        "log: {}",
        std::fs::read_to_string(log_path(world)).unwrap_or_default()
    );
}

#[then(expr = "the persistent runtime should have killed an environment exactly {int} time(s)")]
fn then_killed_times(world: &mut QuectoWorld, n: usize) {
    assert_eq!(
        invocations(world, "kill"),
        n,
        "log: {}",
        std::fs::read_to_string(log_path(world)).unwrap_or_default()
    );
}

#[then(expr = "the persistent runtime should have killed an environment at least {int} time(s)")]
fn then_killed_at_least(world: &mut QuectoWorld, n: usize) {
    assert!(
        invocations(world, "kill") >= n,
        "log: {}",
        std::fs::read_to_string(log_path(world)).unwrap_or_default()
    );
}

#[then(expr = "the persistent runtime should have cleaned up an environment exactly {int} time(s)")]
fn then_cleaned_times(world: &mut QuectoWorld, n: usize) {
    assert_eq!(
        invocations(world, "cleanup"),
        n,
        "log: {}",
        std::fs::read_to_string(log_path(world)).unwrap_or_default()
    );
}

/// The `container ls` row for `env_ref`, split on runs of two or more
/// spaces (the table's column gap).
fn table_row(world: &QuectoWorld, env_ref: &str) -> Option<Vec<String>> {
    world.stdout.lines().find_map(|line| {
        let cells: Vec<String> = line
            .split("  ")
            .filter(|cell| !cell.is_empty())
            .map(|cell| cell.trim().to_string())
            .collect();
        (cells.first().map(String::as_str) == Some(env_ref)).then_some(cells)
    })
}

#[then(
    expr = "the container table should list {string} with name {string} status {string} config {string} and created-by {string}"
)]
fn then_table_lists(
    world: &mut QuectoWorld,
    env_ref: String,
    name: String,
    status: String,
    config: String,
    created_by: String,
) {
    let row = table_row(world, &env_ref)
        .unwrap_or_else(|| panic!("table should list {env_ref}:\n{}", world.stdout));
    // REF NAME CONFIG STATUS REPOSITORY CREATED-BY AGE
    assert_eq!(row[1], name, "{row:?}");
    assert_eq!(row[2], config, "{row:?}");
    assert_eq!(row[3], status, "{row:?}");
    assert_eq!(row[5], created_by, "{row:?}");
}

#[then(expr = "the container table should not list {string}")]
fn then_table_omits(world: &mut QuectoWorld, env_ref: String) {
    assert!(
        table_row(world, &env_ref).is_none(),
        "table should not list {env_ref}:\n{}",
        world.stdout
    );
}

#[then(expr = "the gc report should list {string} as removable")]
fn then_gc_lists_removable(world: &mut QuectoWorld, id: String) {
    let removable = world
        .stdout
        .lines()
        .skip_while(|line| !line.starts_with("would remove") && !line.starts_with("removed"))
        .take_while(|line| !line.starts_with("kept"))
        .any(|line| line.starts_with(&format!("  {id}  ")));
    assert!(
        removable,
        "gc report should list {id} as removable:\n{}",
        world.stdout
    );
}

#[then(expr = "the gc report should keep the environment of {string} as live")]
fn then_gc_keeps(world: &mut QuectoWorld, env_ref: String) {
    let id = environment_id_of(world, &env_ref);
    let kept = world
        .stdout
        .lines()
        .skip_while(|line| !line.starts_with("kept"))
        .any(|line| line.starts_with(&format!("  {id}  ")) && line.contains("is running"));
    assert!(kept, "gc report should keep {id}:\n{}", world.stdout);
}

#[then(
    expr = "the gc report should list the environment of {string} as removable via its retained cleanup"
)]
fn then_gc_lists_retained(world: &mut QuectoWorld, env_ref: String) {
    let id = environment_id_of(world, &env_ref);
    let listed = world.stdout.lines().any(|line| {
        line.starts_with(&format!("  {id}  "))
            && line.contains(&format!("via retained cleanup of {env_ref}"))
    });
    assert!(
        listed,
        "gc report should list {id} via retained cleanup:\n{}",
        world.stdout
    );
}

#[then(expr = "the state dir should still contain {string}")]
fn then_state_dir_contains(world: &mut QuectoWorld, id: String) {
    assert!(
        state_dir(world).join(&id).is_dir(),
        "{id} should still exist"
    );
}

#[then(expr = "the state dir should no longer contain {string}")]
fn then_state_dir_lacks(world: &mut QuectoWorld, id: String) {
    assert!(!state_dir(world).join(&id).exists(), "{id} should be gone");
}

#[then(expr = "the state dir should still contain the environment of {string}")]
fn then_state_dir_contains_env(world: &mut QuectoWorld, env_ref: String) {
    let id = environment_id_of(world, &env_ref);
    assert!(
        state_dir(world).join(&id).is_dir(),
        "{id} should still exist"
    );
}

#[then(expr = "the state dir should no longer contain the environment of {string}")]
fn then_state_dir_lacks_env(world: &mut QuectoWorld, env_ref: String) {
    let id = environment_id_of(world, &env_ref);
    assert!(!state_dir(world).join(&id).exists(), "{id} should be gone");
}

#[then(expr = "the fake runtime should still know container {string}")]
fn then_runtime_knows(world: &mut QuectoWorld, container: String) {
    assert!(
        runtime_dir(world).join(&container).exists(),
        "{container} should still exist"
    );
}

#[then(expr = "the fake runtime should no longer know container {string}")]
fn then_runtime_forgot(world: &mut QuectoWorld, container: String) {
    assert!(
        !runtime_dir(world).join(&container).exists(),
        "{container} should be gone"
    );
}

#[then(expr = "the fake runtime should still know the container of {string}")]
fn then_runtime_knows_env(world: &mut QuectoWorld, env_ref: String) {
    let id = environment_id_of(world, &env_ref);
    let path = runtime_dir(world).join(format!("quecto-{id}"));
    assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), "running");
}
