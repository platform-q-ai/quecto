use super::spawn_env_steps::{
    container_listing_entry, execute_env_spawn, given_shared_script_spawn, run_container_command,
    shared_invocations, shared_log_path, write_executable,
};
use super::*;

// Slice 3 (#1369): direct/proxy liveness and lifecycle parity.
// ===========================================================================
// These steps exercise the production spawn/agent_cmd/monitor stack against
// real script fixtures: proxy bridges are real processes connected to real
// UNIX listeners, and child death is a real `kill -9` of the script-started
// child process (EOF pushed to the parent's liveness connection).

fn cfg_dir(world: &QuectoWorld) -> PathBuf {
    PathBuf::from(world.config_path.clone().unwrap())
        .parent()
        .unwrap()
        .to_path_buf()
}

fn decoy_marker_path(world: &QuectoWorld) -> PathBuf {
    cfg_dir(world).join("decoy.marker")
}

fn load_config(world: &QuectoWorld) -> (PathBuf, serde_json::Value) {
    let cfg_path = PathBuf::from(world.config_path.clone().unwrap());
    let v = serde_json::from_str(&std::fs::read_to_string(&cfg_path).unwrap()).unwrap();
    (cfg_path, v)
}

fn store_config(cfg_path: &PathBuf, v: &serde_json::Value) {
    std::fs::write(cfg_path, serde_json::to_string_pretty(v).unwrap()).unwrap();
}

/// Write the retained `inspect` fixture (logging each invocation) and point
/// the default script set's `inspect` at it.
fn configure_inspect_script(world: &mut QuectoWorld, inspect_fails: bool) {
    let base = base_path(world);
    let log = shared_log_path(world);
    let inspect = base.join("env-inspect.sh");
    let fail_clause = if inspect_fails {
        "echo \"simulated inspect failure\" >&2\nexit 1\n"
    } else {
        ""
    };
    write_executable(
        &inspect,
        format!(
            r#"#!/usr/bin/env bash
set -euo pipefail
echo "{{\"kind\":\"inspect\",\"env_id\":\"${{QUECTO_CONTAINER_ENVIRONMENT_ID:-}}\"}}" >> '{log}'
{fail_clause}printf '{{"status":"dead","metadata":{{"cause":"oom-killed"}}}}'
"#,
            log = log.display()
        ),
    );
    let (cfg_path, mut v) = load_config(world);
    v["container_scripts"]["scripts"]["default"]["inspect"] =
        serde_json::json!([inspect.to_string_lossy()]);
    store_config(&cfg_path, &v);
}

/// Rewrite the default script set's create/exec to variants that log the
/// started child pid + socket, and add the retained `inspect` operation.
fn given_liveness_script_spawn(world: &mut QuectoWorld, inspect_fails: bool) {
    given_shared_script_spawn(world, false);
    let base = base_path(world);
    let log = shared_log_path(world);

    let create = base.join("env-create-live.sh");
    write_executable(
        &create,
        pid_logging_script(&log, "create", "env-live-$RANDOM-$$", true),
    );
    let exec = base.join("env-exec-live.sh");
    write_executable(&exec, pid_logging_script(&log, "exec", "", false));
    configure_inspect_script(world, inspect_fails);

    let (cfg_path, mut v) = load_config(world);
    v["container_scripts"]["scripts"]["default"]["create"] =
        serde_json::json!([create.to_string_lossy()]);
    v["container_scripts"]["scripts"]["default"]["exec"] =
        serde_json::json!([exec.to_string_lossy()]);
    store_config(&cfg_path, &v);
}

/// Shared create/exec fixture body: strip script argv, find the child's
/// `--socket` path, start the child, and log its pid so a later step can kill
/// it behind Quecto's back.
fn pid_logging_script(log: &Path, kind: &str, env_id_expr: &str, is_create: bool) -> String {
    let (env_line, result_line) = if is_create {
        (
            format!(r#"env_id="{env_id_expr}""#),
            r#"printf '{"environment_id":"%s","workspace_path":"%s","metadata":{},"socket_path":"%s"}' "$env_id" "$PWD/workspace-$env_id" "$socket_path""#
                .to_string(),
        )
    } else {
        (
            r#"env_id="${QUECTO_CONTAINER_ENVIRONMENT_ID:-}""#.to_string(),
            r#"printf '{"socket_path":"%s","metadata":{}}' "$socket_path""#.to_string(),
        )
    };
    format!(
        r#"#!/usr/bin/env bash
set -euo pipefail
{env_line}
echo "{{\"kind\":\"{kind}\",\"script\":\"${{QUECTO_CONTAINER_SCRIPT:-}}\",\"env_ref\":\"${{QUECTO_CONTAINER_ENVIRONMENT_REF:-}}\",\"env_id\":\"$env_id\"}}" >> '{log}'
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
echo "{{\"kind\":\"child\",\"pid\":$!,\"socket\":\"$socket_path\"}}" >> '{log}'
{result_line}
"#,
        log = log.display()
    )
}

/// Proxy fixture: the create result carries a validated `socket_proxy` argv
/// (a real stdio<->UDS bridge process) and the child listens on a PRIVATE
/// socket path the parent is never told about directly. When the decoy marker
/// exists, a real decoy listener is bound at the REQUESTED direct socket
/// path; it logs a `decoy-listening` record once bound and a
/// `decoy-connection` record for every connection it receives.
fn given_proxy_script_spawn(world: &mut QuectoWorld) {
    given_shared_script_spawn(world, false);
    let base = base_path(world);
    let log = shared_log_path(world);
    let decoy_marker = decoy_marker_path(world);

    // The bridge program lives in its OWN file: the proxy contract pumps the
    // parent connection over the process's real stdio, so the program must
    // not be fed to python via stdin (a heredoc would steal fd 0).
    let bridge_py = base.join("env-proxy-bridge.py");
    std::fs::write(
        &bridge_py,
        r#"import os, socket, sys, threading
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect(sys.argv[1])
def pump():
    while True:
        d = os.read(0, 65536)
        if not d:
            try:
                s.shutdown(socket.SHUT_WR)
            except OSError:
                pass
            return
        s.sendall(d)
threading.Thread(target=pump, daemon=True).start()
while True:
    d = s.recv(65536)
    if not d:
        break
    os.write(1, d)
"#,
    )
    .unwrap();
    let proxy = base.join("env-proxy.sh");
    let proxy_script = r#"#!/usr/bin/env bash
set -euo pipefail
echo "{\"kind\":\"proxy\",\"target\":\"$1\"}" >> '__LOG__'
exec python3 '__BRIDGE__' "$1"
"#
    .replace("__LOG__", &log.display().to_string())
    .replace("__BRIDGE__", &bridge_py.display().to_string());
    write_executable(&proxy, proxy_script);

    let create = base.join("env-create-proxy.sh");
    let create_script = r#"#!/usr/bin/env bash
set -euo pipefail
env_id="env-proxy-$RANDOM-$$"
echo "{\"kind\":\"create\",\"script\":\"${QUECTO_CONTAINER_SCRIPT:-}\",\"env_ref\":\"${QUECTO_CONTAINER_ENVIRONMENT_REF:-}\",\"env_id\":\"$env_id\"}" >> '__LOG__'
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--" ]; then shift; break; fi
  shift
done
requested=""
new_args=()
prev=""
for arg in "$@"; do
  if [ "$prev" = "--socket" ]; then requested="$arg"; fi
  prev="$arg"
done
if [ -z "$requested" ]; then echo "no --socket in child argv" >&2; exit 1; fi
# Keep the private path short (and under the 104-byte UDS limit) while still
# embedding the agent UUID from the requested basename.
private_sock="${TMPDIR:-/tmp}/pq-$(basename "$requested")"
new_args=()
prev=""
for arg in "$@"; do
  if [ "$prev" = "--socket" ]; then new_args+=("$private_sock"); else new_args+=("$arg"); fi
  prev="$arg"
done
if [ -e '__DECOY_MARKER__' ]; then
  python3 - "$requested" '__LOG__' <<'PY' >/dev/null 2>&1 &
import json, os, socket, sys
path, log = sys.argv[1], sys.argv[2]
try:
    os.unlink(path)
except FileNotFoundError:
    pass
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(path)
s.listen(8)
with open(log, "a") as f:
    f.write(json.dumps({"kind": "decoy-listening", "path": path}) + "\n")
while True:
    c, _ = s.accept()
    with open(log, "a") as f:
        f.write(json.dumps({"kind": "decoy-connection", "path": path}) + "\n")
    c.close()
PY
fi
"${new_args[@]}" >/dev/null 2>&1 &
echo "{\"kind\":\"child\",\"pid\":$!,\"socket\":\"$private_sock\"}" >> '__LOG__'
printf '{"environment_id":"%s","workspace_path":"%s","metadata":{},"socket_proxy":{"argv":["__PROXY__","%s"]}}' "$env_id" "$PWD/workspace-$env_id" "$private_sock"
"#
    .replace("__LOG__", &log.display().to_string())
    .replace("__DECOY_MARKER__", &decoy_marker.display().to_string())
    .replace("__PROXY__", &proxy.display().to_string());
    write_executable(&create, create_script);

    let (cfg_path, mut v) = load_config(world);
    v["container_scripts"]["scripts"]["default"]["create"] =
        serde_json::json!([create.to_string_lossy()]);
    store_config(&cfg_path, &v);
}

// --- Given ---

#[given("liveness script-managed subagent spawning is available")]
fn given_liveness_spawn(world: &mut QuectoWorld) {
    given_liveness_script_spawn(world, false);
}

#[given("liveness script-managed subagent spawning is available with an inspect script that fails")]
fn given_liveness_spawn_inspect_fails(world: &mut QuectoWorld) {
    given_liveness_script_spawn(world, true);
}

#[given("proxy-capable script-managed subagent spawning is available")]
fn given_proxy_spawn(world: &mut QuectoWorld) {
    given_proxy_script_spawn(world);
}

#[given("proxy-capable liveness script-managed subagent spawning is available")]
fn given_proxy_liveness_spawn(world: &mut QuectoWorld) {
    given_proxy_script_spawn(world);
    configure_inspect_script(world, false);
}

#[given("a decoy direct socket is planted at the requested child socket path")]
fn given_decoy_direct_socket(world: &mut QuectoWorld) {
    std::fs::write(decoy_marker_path(world), b"decoy").unwrap();
}

#[given("the script-managed create result carries both a socket path and a socket proxy")]
fn given_create_result_both_endpoints(world: &mut QuectoWorld) {
    let base = base_path(world);
    let log = shared_log_path(world);
    let both = base.join("env-create-both.sh");
    let script = r#"#!/usr/bin/env bash
set -euo pipefail
env_id="env-both-$$"
echo "{\"kind\":\"create\",\"script\":\"${QUECTO_CONTAINER_SCRIPT:-}\",\"env_ref\":\"${QUECTO_CONTAINER_ENVIRONMENT_REF:-}\",\"env_id\":\"$env_id\"}" >> '__LOG__'
printf '{"environment_id":"%s","workspace_path":"%s","metadata":{},"socket_path":"%s","socket_proxy":{"argv":["/bin/true"]}}' "$env_id" "$PWD/workspace-$env_id" "$PWD/never-used.sock"
"#
    .replace("__LOG__", &log.display().to_string());
    write_executable(&both, script);
    let (cfg_path, mut v) = load_config(world);
    v["container_scripts"]["scripts"]["default"]["create"] =
        serde_json::json!([both.to_string_lossy()]);
    store_config(&cfg_path, &v);
}

#[given(
    expr = "script-managed child {string} is running in an inspectable environment with task {string}"
)]
fn given_inspectable_child_running(world: &mut QuectoWorld, agent_id: String, task: String) {
    execute_env_spawn(
        world,
        &agent_id,
        serde_json::json!({"agent_id":agent_id,"task":task,"container":true,"read_only":true}),
    );
    let r = world.spawn_result.as_ref().unwrap();
    assert!(!r.is_error, "inspectable spawn failed: {}", r.content);
}

#[given(
    expr = "script-managed child {string} is running in a proxy-only environment with task {string}"
)]
fn given_proxy_child_running(world: &mut QuectoWorld, agent_id: String, task: String) {
    when_spawn_proxy_only(world, agent_id, task);
    let r = world.spawn_result.as_ref().unwrap();
    assert!(!r.is_error, "proxy spawn failed: {}", r.content);
}

#[given(expr = "subagent {string} has already exited behind Quecto's back")]
fn given_already_exited_behind_back(world: &mut QuectoWorld, agent_id: String) {
    when_child_killed_behind_back(world, agent_id.clone());
    // Context step: the death must be fully observed (await returns exited)
    // before the scenario's own trigger fires.
    then_await_reports_status(world, agent_id, "exited".to_string());
}

#[given(
    expr = "the environment registry entry {string} has been removed out from under the monitor"
)]
fn given_environment_entry_removed(world: &mut QuectoWorld, env_ref: String) {
    // Simulate the registry-removal race: monitoring must keep working via
    // the endpoint the monitor captured at launch, not the registry entry.
    let removed = world
        .spawn_tool
        .as_ref()
        .expect("spawn tool")
        .environment_registry()
        .remove(&env_ref);
    assert!(removed.is_some(), "no committed environment {env_ref}");
}

// --- When ---

#[when(
    expr = "I spawn script-managed subagent {string} into a new proxy-only environment with task {string}"
)]
fn when_spawn_proxy_only(world: &mut QuectoWorld, agent_id: String, task: String) {
    execute_env_spawn(
        world,
        &agent_id,
        serde_json::json!({"agent_id":agent_id,"task":task,"container":true,"read_only":true}),
    );
}

#[when(expr = "the script-managed child {string} is killed behind Quecto's back")]
fn when_child_killed_behind_back(world: &mut QuectoWorld, agent_id: String) {
    let uuid = world
        .agent_spawn_uuids
        .get(&agent_id)
        .unwrap_or_else(|| {
            panic!(
                "no captured uuid for {agent_id}: {:?}",
                world.agent_spawn_uuids
            )
        })
        .clone();
    // The fixture logs every script-started child with the socket path it
    // serves; both direct and proxy-private paths embed the agent UUID.
    let inv = shared_invocations(world);
    let pid = inv
        .iter()
        .filter(|v| v["kind"] == "child")
        .find(|v| {
            v["socket"]
                .as_str()
                .is_some_and(|socket| socket.contains(uuid.as_str()))
        })
        .and_then(|v| v["pid"].as_i64())
        .unwrap_or_else(|| panic!("no logged child pid for agent {agent_id} ({uuid}): {inv:?}"));
    let status = std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status()
        .expect("invoke kill");
    assert!(status.success(), "kill -9 {pid} failed");
}

// --- Then ---

#[then(
    expr = "the script-managed runtime should have inspected an environment exactly {int} time(s)"
)]
fn then_inspect_count(world: &mut QuectoWorld, n: i32) {
    let inv = shared_invocations(world);
    let inspects: Vec<&serde_json::Value> = inv.iter().filter(|v| v["kind"] == "inspect").collect();
    assert_eq!(inspects.len() as i32, n, "invocations: {inv:?}");
    // Every inspect must target an environment id this session created.
    let created_ids: Vec<&str> = inv
        .iter()
        .filter(|v| v["kind"] == "create")
        .filter_map(|v| v["env_id"].as_str())
        .collect();
    for inspect in inspects {
        let env_id = inspect["env_id"].as_str().unwrap_or_default();
        assert!(
            !env_id.is_empty() && created_ids.contains(&env_id),
            "inspect must target a created environment id: inspect={inspect} created={created_ids:?}"
        );
    }
}

#[then(expr = "the proxy bridge should have been used at least {int} time(s)")]
fn then_proxy_bridge_used(world: &mut QuectoWorld, n: i32) {
    let inv = shared_invocations(world);
    let count = inv.iter().filter(|v| v["kind"] == "proxy").count() as i32;
    assert!(count >= n, "expected >= {n} proxy bridge uses: {inv:?}");
}

#[then("the decoy direct socket should have been listening yet received no connections")]
fn then_decoy_untouched(world: &mut QuectoWorld) {
    // The decoy must provably have been planted AND bound at the requested
    // direct child socket path — otherwise "no connections" is vacuous.
    let uuid = world
        .agent_spawn_uuids
        .get("proxy-decoy-slice3")
        .expect("captured decoy-scenario uuid")
        .clone();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let inv = shared_invocations(world);
        if let Some(listening) = inv.iter().find(|v| v["kind"] == "decoy-listening") {
            let path = listening["path"].as_str().unwrap_or_default();
            assert!(
                path.contains(uuid.as_str()),
                "decoy must be bound at the requested direct socket path for {uuid}: {listening}"
            );
            let connections = inv
                .iter()
                .filter(|v| v["kind"] == "decoy-connection")
                .count();
            assert_eq!(
                connections, 0,
                "proxy mode fell back to the decoy direct socket: {inv:?}"
            );
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "decoy listener never reported listening: {inv:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

#[then("the spawn result should fail because the create result must carry exactly one endpoint")]
fn then_spawn_fails_exactly_one_endpoint(world: &mut QuectoWorld) {
    let r = world.spawn_result.as_ref().unwrap();
    assert!(
        r.is_error && r.content.contains("exactly one"),
        "expected exactly-one-endpoint failure: {}",
        r.content
    );
}

#[then(
    expr = "the script-managed runtime should have cleaned up an environment exactly {int} time(s)"
)]
fn then_env_cleanup_count(world: &mut QuectoWorld, n: i32) {
    let inv = shared_invocations(world);
    let count = inv.iter().filter(|v| v["kind"] == "cleanup").count() as i32;
    assert_eq!(count, n, "invocations: {inv:?}");
}

#[then(
    expr = "the container listing entry {string} should carry inspect metadata {string} with value {string}"
)]
fn then_listing_inspect_metadata(
    world: &mut QuectoWorld,
    env_ref: String,
    key: String,
    value: String,
) {
    let entry = container_listing_entry(world, &env_ref);
    assert_eq!(
        entry["metadata"][&key].as_str(),
        Some(value.as_str()),
        "inspect result must update the authoritative environment metadata: {entry}"
    );
}

#[then(expr = "the container listing entry {string} should record an inspect error")]
fn then_listing_inspect_error(world: &mut QuectoWorld, env_ref: String) {
    let entry = container_listing_entry(world, &env_ref);
    let last_error = entry["last_error"].as_str().unwrap_or_default();
    assert!(
        last_error.contains("inspect"),
        "inspect failure must persist an actionable last error: {entry}"
    );
}

#[then(expr = "a passive exit note for {string} should be delivered")]
fn then_passive_exit_note(world: &mut QuectoWorld, agent_id: String) {
    let mut rx = world.notify_rx.take().expect("notification rx wired");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match rx.try_recv() {
            Ok(sequenced) => {
                let message = sequenced.notification.to_message();
                if message.contains(&agent_id) && message.to_lowercase().contains("exit") {
                    world.notify_rx = Some(rx);
                    return;
                }
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "no passive exit note for {agent_id} arrived"
                );
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(e) => panic!("notification channel closed: {e}"),
        }
    }
}

#[then(expr = "the live event stream should report subagent {string} as exited")]
fn then_live_event_reports_exited(world: &mut QuectoWorld, agent_id: String) {
    let rx = world
        .spawn_broadcast_rx
        .as_mut()
        .expect("broadcast rx wired");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match rx.try_recv() {
            Ok(event) => {
                if event.contains("subagent_state_changed")
                    && event.contains(&agent_id)
                    && event.contains("exited")
                {
                    return;
                }
            }
            Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "no live state event reported {agent_id} as exited"
                );
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {}
            Err(e) => panic!("broadcast channel closed: {e}"),
        }
    }
}

#[then(expr = "the subagent snapshot should report {string} as exited")]
fn then_snapshot_reports_exited(world: &mut QuectoWorld, agent_id: String) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let result = run_container_command(
            world,
            serde_json::json!({"agent_id": "*", "command": "get_subagents_all"}),
        );
        assert!(!result.is_error, "snapshot failed: {}", result.content);
        let parsed: serde_json::Value = serde_json::from_str(&result.content)
            .unwrap_or_else(|e| panic!("snapshot must be JSON: {e}; got {}", result.content));
        let exited = parsed["subagents"].as_array().is_some_and(|subagents| {
            subagents.iter().any(|s| {
                s["agentId"].as_str() == Some(agent_id.as_str())
                    && s["status"].as_str() == Some("exited")
            })
        });
        if exited {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "snapshot never reported {agent_id} as exited: {parsed}"
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
