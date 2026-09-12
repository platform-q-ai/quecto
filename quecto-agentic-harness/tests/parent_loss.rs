//! Real-process parent loss (#1935): a launcher process that created a child
//! through the production `SpawnTool` is SIGKILLed; the child must exit
//! within a bound, over the direct UDS transport and over the proxy
//! (container) transport alike.
//!
//! The launcher is this test binary re-executed with `QUECTO_PARENT_LOSS_ROLE`
//! set, so the parent side runs the real launch path: credential minting,
//! the private sidecar, the `--parent-control` argv, and the monitor's
//! presentation on the bound connection. The child is the real `quecto`
//! binary. Only the launcher is killed; the child must notice through its
//! bound connection (or, on Linux, the parent-death signal as defence in
//! depth — over the proxy transport the child is not even the launcher's
//! process child, so only the connection can tell it).
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const ROLE_ENV: &str = "QUECTO_PARENT_LOSS_ROLE";
const BASE_ENV: &str = "QUECTO_PARENT_LOSS_BASE";
const READY_TIMEOUT: Duration = Duration::from_secs(90);
const EXIT_BOUND: Duration = Duration::from_secs(20);

fn alive(pid: u32) -> bool {
    // SAFETY: signal 0 only probes existence of the given pid.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

fn write_executable(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Shared config for the child: an unreachable provider (the child never
/// needs a completion to be alive) and, for the proxy transport, a create
/// script that starts the child detached and hands back a proxy argv.
fn write_config(base: &Path, transport: &str) -> PathBuf {
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut config = serde_json::json!({
        "providers": {"openai": {"api_key": "sk-test", "api_base": "http://127.0.0.1:9"}},
        "agents": {"defaults": {"model": "openai-api/gpt-4o-mini", "workspace": workspace}}
    });
    if transport == "proxy" {
        let bridge = base.join("bridge.py");
        std::fs::write(
            &bridge,
            r#"import os, socket, sys, threading
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect(sys.argv[1])
def pump():
    while True:
        d = os.read(0, 65536)
        if not d:
            # The parent side is gone: drop the whole connection so the
            # child sees its bound connection lost, then leave.
            try:
                s.close()
            finally:
                os._exit(0)
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
        let create = base.join("create.sh");
        let script = r#"#!/usr/bin/env bash
set -euo pipefail
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--" ]; then shift; break; fi
  shift
done
requested=""
prev=""
for arg in "$@"; do
  if [ "$prev" = "--socket" ]; then requested="$arg"; fi
  prev="$arg"
done
private_sock="${TMPDIR:-/tmp}/pl-$(basename "$requested")"
new_args=()
prev=""
for arg in "$@"; do
  if [ "$prev" = "--socket" ]; then new_args+=("$private_sock"); else new_args+=("$arg"); fi
  prev="$arg"
done
# Detached: the child is NOT a process child of the launcher, so only the
# bound connection through the proxy can tell it the launcher is gone.
setsid "${new_args[@]}" >/dev/null 2>&1 < /dev/null &
echo "$!" > '__BASE__/child.pid'
printf '{"environment_id":"env-1","workspace_path":"%s","metadata":{},"socket_proxy":{"argv":["python3","__BRIDGE__","%s"]}}' "$PWD" "$private_sock"
"#
        .replace("__BASE__", &base.display().to_string())
        .replace("__BRIDGE__", &bridge.display().to_string());
        write_executable(&create, &script);
        config["container_configs"] = serde_json::json!({
            "default": {
                "default": true,
                "create": [create.display().to_string()],
                "cleanup": ["true"],
            }
        });
    }
    let path = base.join("config.json");
    std::fs::write(&path, serde_json::to_string_pretty(&config).unwrap()).unwrap();
    path
}

/// The launcher role: launch one child through the production `SpawnTool`,
/// publish what the test needs, then wait to be killed.
#[test]
fn launcher_role() {
    let Ok(transport) = std::env::var(ROLE_ENV) else {
        return;
    };
    let base = PathBuf::from(std::env::var(BASE_ENV).expect("launcher base dir"));
    let config = write_config(&base, &transport);
    let sockets = base.join("sockets");
    std::fs::create_dir_all(&sockets).unwrap();
    // The child binary must be the real quecto, not this test executable.
    // SAFETY: single-threaded at this point, so the env write cannot race.
    unsafe { std::env::set_var("QUECTO_CHILD_BINARY", env!("CARGO_BIN_EXE_quecto")) };
    let registry = quecto::infrastructure::tools::agent_cmd::AgentCmdTool::new_registry();
    let tool = quecto::infrastructure::tools::spawn::SpawnTool::with_base_dir(vec![], base.clone())
        .with_socket_dir(sockets.clone())
        .with_registry(registry.clone())
        .with_parent_config_path(Some(config.clone()));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let args = serde_json::json!({
        "agent_id": "bound-child",
        "task": "wait",
        "config": config,
        "read_only": true,
        "container": transport == "proxy",
    });
    let result = runtime
        .block_on(quecto::domain::tool::Tool::execute(
            &tool,
            &args.to_string(),
        ))
        .expect("spawn tool ran");
    assert!(!result.is_error, "spawn failed: {}", result.content);
    let (pid, socket_path) = {
        let entries = registry.lock().unwrap();
        let entry = entries.values().next().expect("one child registered");
        (entry.pid, entry.socket_path.clone())
    };
    let pid = if transport == "proxy" {
        std::fs::read_to_string(base.join("child.pid"))
            .unwrap()
            .trim()
            .parse::<u32>()
            .unwrap()
    } else {
        pid
    };
    // The sidecar was consumed by the child: no capability material remains.
    let leftovers: Vec<_> = std::fs::read_dir(&sockets)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("quecto-parent-control")
        })
        .collect();
    assert!(
        leftovers.is_empty(),
        "sidecar must be consumed: {leftovers:?}"
    );
    std::fs::write(base.join("child.socket"), socket_path.display().to_string()).unwrap();
    std::fs::write(base.join("child.pid.final"), pid.to_string()).unwrap();
    std::fs::write(base.join("ready"), b"ready").unwrap();
    // Keep the runtime (monitor task, bound connection) alive until killed.
    std::thread::sleep(Duration::from_secs(3600));
    drop(runtime);
}

fn run_case(transport: &str) {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().to_path_buf();
    let mut launcher = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "launcher_role",
            "--nocapture",
            "--test-threads",
            "1",
        ])
        .env(ROLE_ENV, transport)
        .env(BASE_ENV, &base)
        .env("QUECTO_BASE_DIR", &base)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("re-exec launcher");
    let ready = base.join("ready");
    let started = Instant::now();
    while !ready.exists() {
        if let Some(status) = launcher.try_wait().unwrap() {
            let output = launcher.wait_with_output().unwrap();
            panic!(
                "launcher exited early ({status}):\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        assert!(
            started.elapsed() < READY_TIMEOUT,
            "launcher never became ready"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    let child_pid: u32 = std::fs::read_to_string(base.join("child.pid.final"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let socket_path = PathBuf::from(std::fs::read_to_string(base.join("child.socket")).unwrap());
    assert!(
        alive(child_pid),
        "child must be alive before the parent dies"
    );
    // Kill the launcher outright: no signal handler, no orderly teardown.
    // SAFETY: the pid is the launcher this test just spawned.
    unsafe { libc::kill(launcher.id() as libc::pid_t, libc::SIGKILL) };
    let _ = launcher.wait();
    let killed = Instant::now();
    while alive(child_pid) {
        if killed.elapsed() > EXIT_BOUND {
            // SAFETY: best-effort cleanup of the pid this test observed.
            unsafe { libc::kill(child_pid as libc::pid_t, libc::SIGKILL) };
            panic!("child {child_pid} outlived its launcher over the {transport} transport");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if transport == "direct" {
        // A graceful exit removes the child's socket file (the socket guard
        // runs on the loop's exit path); a hard kill would leave it behind.
        let deadline = Instant::now() + Duration::from_secs(5);
        while socket_path.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            !socket_path.exists(),
            "child socket {} must be removed by a graceful exit",
            socket_path.display()
        );
    }
}

/// A launcher that writes the sidecar and starts the child but never
/// presents (it died, or hung, between spawning and binding): the child
/// must not stay an unbound `--persist` orphan. The bind deadline ends it,
/// gracefully, with reason `parent_never_bound`.
#[test]
fn a_child_whose_parent_never_binds_exits_at_the_bind_deadline() {
    use quecto::infrastructure::processes::parent_control::{mint_credential, write_sidecar};
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().to_path_buf();
    let config = write_config(&base, "direct");
    let sockets = base.join("s");
    std::fs::create_dir_all(&sockets).unwrap();
    let sidecar = sockets.join("quecto-parent-control-never");
    write_sidecar(&sidecar, &mint_credential()).unwrap();
    let socket_path = sockets.join("never-bound.sock");
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_quecto"))
        .args(["agent", "--mode", "uds", "-s", "never-bound", "--socket"])
        .arg(&socket_path)
        .args(["--persist", "--spawned", "--config"])
        .arg(&config)
        .arg("--parent-control")
        .arg(&sidecar)
        .env("QUECTO_BASE_DIR", &base)
        .env(
            quecto::interface::cli::uds_teardown_graph::BIND_DEADLINE_ENV,
            "1000",
        )
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn the child");
    let started = Instant::now();
    while !socket_path.exists() {
        if let Some(status) = child.try_wait().unwrap() {
            let mut stderr = String::new();
            std::io::Read::read_to_string(child.stderr.as_mut().unwrap(), &mut stderr).unwrap();
            panic!("child exited before binding its socket ({status}): {stderr}");
        }
        assert!(
            started.elapsed() < READY_TIMEOUT,
            "child socket never appeared"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!sidecar.exists(), "the child consumed the sidecar");
    let bound = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if bound.elapsed() > EXIT_BOUND {
            let _ = child.kill();
            panic!("an unbound child outlived its bind deadline");
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(
        status.success(),
        "the deadline runs the common shutdown, a graceful exit: {status}"
    );
    assert!(
        bound.elapsed() >= Duration::from_millis(900),
        "the child must wait for the deadline, not exit at once"
    );
    assert!(!socket_path.exists(), "a graceful exit removes the socket");
}

#[test]
fn parent_sigkill_ends_the_child_over_direct_uds() {
    run_case("direct");
}

#[test]
fn parent_sigkill_ends_the_detached_child_over_the_proxy_transport() {
    run_case("proxy");
}
