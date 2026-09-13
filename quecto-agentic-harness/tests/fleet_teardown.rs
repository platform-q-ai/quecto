//! Real-process fleet teardown (#1938, epic #1929): a top-level `quecto`
//! harness process launches a real child through its production spawn
//! tool, and every exit/transition entry point tears that child down through
//! the one fleet teardown before the harness returns or continues:
//!
//! - SIGTERM and SIGINT: the child is gone before the harness exits 0, and
//!   the session is persisted without a live child;
//! - the last client's disconnect of the default lifetime: teardown, then
//!   exit; a top-level `--persist` harness survives its last client;
//! - `new_session` and `resume_session`: the child is gone before the roster
//!   is replaced, and the harness keeps serving;
//! - a container (proxy-transport) child with a fake retained kill argv: the
//!   session switch settles it and runs the argv once.
//!
//! Every wait is bounded so a regression fails instead of hanging.
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Bound on every wait. Generous for a 2-vCPU CI runner where six harness
/// processes, their children, bridges and mock providers share the cores.
const BOUND: Duration = Duration::from_secs(120);

fn alive(pid: u32) -> bool {
    // SAFETY: signal 0 only probes existence of the given pid.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

fn write_executable(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A provider that makes the harness spawn one child ("worker") when the
/// prompt says `SPAWN_ONE`, then finish; every other turn just answers.
async fn provider(config: PathBuf, container: bool) -> wiremock::MockServer {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(move |request: &wiremock::Request| {
            let input: serde_json::Value = request.body_json().unwrap();
            let messages = input["messages"].as_array().cloned().unwrap_or_default();
            let is_spawner = messages.iter().any(|m| {
                m["role"] == "user"
                    && m["content"]
                        .as_str()
                        .is_some_and(|content| content.contains("SPAWN_ONE"))
            });
            let tool_results = messages.iter().filter(|m| m["role"] == "tool").count();
            let delta = if is_spawner && tool_results == 0 {
                let arguments = serde_json::json!({
                    "agent_id": "worker",
                    "task": "wait",
                    "read_only": true,
                    "config": config,
                    "container": container,
                })
                .to_string();
                serde_json::json!({"tool_calls": [{
                    "index": 0,
                    "id": "call-worker",
                    "type": "function",
                    "function": {"name": "spawn", "arguments": arguments}
                }]})
            } else {
                serde_json::json!({"content": "DONE"})
            };
            let finish = if delta.get("tool_calls").is_some() {
                "tool_calls"
            } else {
                "stop"
            };
            let chunk = serde_json::json!({
                "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
                "usage": {"prompt_tokens": 10, "completion_tokens": 3, "total_tokens": 13}
            });
            wiremock::ResponseTemplate::new(200).set_body_raw(
                format!("data: {chunk}\n\ndata: [DONE]\n\n"),
                "text/event-stream",
            )
        })
        .mount(&server)
        .await;
    server
}

struct Fixture {
    _dir: tempfile::TempDir,
    base: PathBuf,
    config: PathBuf,
    _server: wiremock::MockServer,
    runtime: tokio::runtime::Runtime,
}

/// Base dir, config and provider. With `container`, the config carries a
/// container set whose create script starts the child detached behind a
/// proxy bridge and whose kill script records into `killed.log`.
fn fixture(container: bool) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().to_path_buf();
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let config = base.join("config.json");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(provider(config.clone(), container));
    let mut json = serde_json::json!({
        "providers": {"openai": {"api_key": "sk-test", "api_base": server.uri()}},
        "agents": {"defaults": {"model": "openai-api/gpt-4o-mini", "workspace": workspace}}
    });
    if container {
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
private_sock="${TMPDIR:-/tmp}/ft-$(basename "$requested")"
new_args=()
prev=""
for arg in "$@"; do
  if [ "$prev" = "--socket" ]; then new_args+=("$private_sock"); else new_args+=("$arg"); fi
  prev="$arg"
done
setsid "${new_args[@]}" >/dev/null 2>&1 < /dev/null &
echo "$!" > '__BASE__/child.pid'
printf '{"environment_id":"env-1","workspace_path":"%s","metadata":{},"socket_proxy":{"argv":["python3","__BRIDGE__","%s"]}}' "$PWD" "$private_sock"
"#
        .replace("__BASE__", &base.display().to_string())
        .replace("__BRIDGE__", &bridge.display().to_string());
        write_executable(&create, &script);
        let kill = base.join("kill.sh");
        write_executable(
            &kill,
            &format!(
                "#!/bin/sh\nprintf 'killed %s\\n' \"$QUECTO_CONTAINER_ENVIRONMENT_ID\" >> '{}'\n",
                base.join("killed.log").display()
            ),
        );
        json["container_configs"] = serde_json::json!({
            "default": {
                "default": true,
                "create": [create.display().to_string()],
                "kill": [kill.display().to_string()],
                "cleanup": ["true"],
            }
        });
    }
    std::fs::write(&config, serde_json::to_string_pretty(&json).unwrap()).unwrap();
    Fixture {
        _dir: dir,
        base,
        config,
        _server: server,
        runtime,
    }
}

struct Harness {
    child: std::process::Child,
    socket: PathBuf,
}

impl Harness {
    fn start(fixture: &Fixture, session: &str, persist: bool) -> Self {
        let socket = fixture.base.join(format!("{session}.sock"));
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_quecto"));
        command
            .args(["agent", "--mode", "uds", "--socket"])
            .arg(&socket)
            .args(["-s", session, "--config"])
            .arg(&fixture.config);
        if persist {
            command.arg("--persist");
        }
        let child = command
            .env("QUECTO_BASE_DIR", &fixture.base)
            .env("QUECTO_CHILD_BINARY", env!("CARGO_BIN_EXE_quecto"))
            .env("HOME", &fixture.base)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .expect("spawn uds harness");
        let mut harness = Self { child, socket };
        let deadline = Instant::now() + BOUND;
        while !harness.socket.exists() {
            if let Some(status) = harness.child.try_wait().unwrap() {
                panic!("harness exited before binding its socket: {status}");
            }
            assert!(Instant::now() < deadline, "harness never bound");
            std::thread::sleep(Duration::from_millis(50));
        }
        harness
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn connect(&self) -> Client {
        let stream = UnixStream::connect(&self.socket).expect("connect");
        stream.set_read_timeout(Some(BOUND)).unwrap();
        Client { stream }
    }

    fn signal(&self, signal: &str) {
        let status = std::process::Command::new("kill")
            .args([signal, &self.pid().to_string()])
            .status()
            .unwrap();
        assert!(status.success());
    }

    /// The harness exits on its own, cleanly, within the bound.
    fn wait_exit(&mut self) -> std::process::ExitStatus {
        let deadline = Instant::now() + BOUND;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                panic!("harness did not exit within the bound");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn still_running(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }

    /// Observe the child gone **while this harness is still running**: the
    /// fleet settled it before the exit path. A harness that skipped the
    /// fleet step would exit first and the child would only die afterwards
    /// by parent loss, which this ordering refuses. Returns the exit status
    /// once the harness has returned on its own.
    fn child_gone_then_exit(&mut self, child: u32) -> std::process::ExitStatus {
        let deadline = Instant::now() + BOUND;
        loop {
            let running = self.still_running();
            let gone = !alive(child);
            match (gone, running) {
                (true, true) => break,
                (false, false) => panic!("the harness exited while its child was still alive"),
                (true, false) => panic!(
                    "the child was only observed gone after the harness had exited; the fleet must settle it first"
                ),
                (false, true) => {}
            }
            assert!(Instant::now() < deadline, "the child never settled");
            std::thread::sleep(Duration::from_millis(1));
        }
        self.wait_exit()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Client {
    stream: UnixStream,
}

impl Client {
    fn send(&mut self, line: &str) {
        self.stream.write_all(line.as_bytes()).unwrap();
        self.stream.write_all(b"\n").unwrap();
    }

    /// Read newline-delimited JSON until one matches, within the bound.
    /// `what` names the awaited event in a failure, with the events seen.
    fn read_until(
        &mut self,
        what: &str,
        want: impl Fn(&serde_json::Value) -> bool,
    ) -> serde_json::Value {
        let deadline = Instant::now() + BOUND;
        let mut buffer = Vec::new();
        let mut byte = [0u8; 1];
        let mut seen: Vec<String> = Vec::new();
        loop {
            assert!(
                Instant::now() < deadline,
                "no {what} within the bound; events seen: {seen:?}"
            );
            match self.stream.read(&mut byte) {
                Ok(0) => panic!("harness closed the connection while waiting"),
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                Err(e) => panic!("read failed: {e}"),
            }
            if byte[0] == b'\n' {
                if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&buffer) {
                    if want(&value) {
                        return value;
                    }
                    seen.push(
                        value["type"]
                            .as_str()
                            .or_else(|| value["command"].as_str())
                            .unwrap_or("?")
                            .to_owned(),
                    );
                }
                buffer.clear();
            } else {
                buffer.push(byte[0]);
            }
        }
    }

    fn request(&mut self, line: &str, id: &str) -> serde_json::Value {
        self.send(line);
        self.read_until(&format!("response {id}"), |v| v["id"] == id)
    }

    /// Prompt the harness to spawn one child and wait until the child is
    /// registered live and the turn is over. Returns the child's pid as the
    /// harness reports it: the process pid for a local child, whatever the
    /// launch recorded (possibly 0) for a container child, whose real pid
    /// the create script publishes.
    fn spawn_worker(&mut self, local: bool) -> u32 {
        self.send(r#"{"type":"prompt","message":"SPAWN_ONE","id":"p1"}"#);
        // The spawn tool returned: the child is registered (its pid known
        // for a local child) and the turn is over. A container child's
        // status is not awaited here — its liveness is proven through the
        // pid the create script published — so a slow bridge on a loaded
        // runner cannot stall the test before the teardown it exercises.
        let event = self.read_until("registered worker", |v| {
            v["type"] == "subagent_state_changed"
                && v["subagents"].as_array().is_some_and(|rows| {
                    rows.iter().any(|row| {
                        (!local || row["pid"].as_u64().unwrap_or(0) > 0)
                            && row["status"] != "exited"
                    })
                })
        });
        let pid = event["subagents"][0]["pid"].as_u64().unwrap_or(0) as u32;
        self.read_until("agent_end", |v| v["type"] == "agent_end");
        pid
    }
}

fn wait_gone(pid: u32, what: &str) {
    let started = Instant::now();
    while alive(pid) {
        if started.elapsed() > BOUND {
            // SAFETY: best-effort cleanup of the pid this test observed.
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
            panic!("{what} ({pid}) did not exit within the bound");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The persisted session of `session` carries no live operational child.
fn assert_no_live_child_persisted(fixture: &Fixture, session: &str) {
    use quecto::domain::session::{Session, SessionStore, SubagentLiveness};
    let store =
        quecto::infrastructure::persistence::session_store::FileSessionStore::new(&fixture.base);
    let key = Session::build_key("cli", session);
    let saved = fixture
        .runtime
        .block_on(store.load(&key))
        .unwrap()
        .expect("the session was persisted on the way out");
    assert!(
        saved
            .subagent_roster
            .iter()
            .all(|row| row.liveness != SubagentLiveness::Live),
        "a live child was persisted: {:?}",
        saved.subagent_roster
    );
}

fn signal_tears_down_then_exits(signal: &str) {
    let fixture = fixture(false);
    let mut harness = Harness::start(&fixture, "signal", false);
    let mut client = harness.connect();
    let worker = client.spawn_worker(true);
    assert!(alive(worker));

    harness.signal(signal);
    // The child settles while the harness still runs, then the harness exits.
    let status = harness.child_gone_then_exit(worker);
    assert_eq!(status.signal(), None, "handled, not killed: {status}");
    assert_eq!(status.code(), Some(0), "{status}");
    assert_no_live_child_persisted(&fixture, "signal");
}

#[test]
fn sigterm_tears_down_the_live_child_before_the_harness_exits() {
    signal_tears_down_then_exits("-TERM");
}

#[test]
fn sigint_tears_down_the_live_child_before_the_harness_exits() {
    signal_tears_down_then_exits("-INT");
}

#[test]
fn the_last_client_disconnect_of_the_default_lifetime_tears_down_then_exits() {
    let fixture = fixture(false);
    let mut harness = Harness::start(&fixture, "last-client", false);
    let mut client = harness.connect();
    let worker = client.spawn_worker(true);
    assert!(alive(worker));
    drop(client);
    let status = harness.child_gone_then_exit(worker);
    assert_eq!(status.code(), Some(0), "{status}");
    assert_no_live_child_persisted(&fixture, "last-client");
}

#[test]
fn a_top_level_persist_harness_survives_its_last_client() {
    let fixture = fixture(false);
    let mut harness = Harness::start(&fixture, "persist", true);
    let mut client = harness.connect();
    let worker = client.spawn_worker(true);
    drop(client);
    std::thread::sleep(Duration::from_secs(2));
    assert!(harness.still_running(), "--persist ignores its last client");
    assert!(alive(worker), "the child of a persistent harness lives on");
    // A fresh client is served as before.
    let mut again = harness.connect();
    let state = again.request(r#"{"type":"get_state","id":"s1"}"#, "s1");
    assert_eq!(state["success"], true, "{state}");
    // Only an explicit shutdown ends it, tearing the child down first.
    harness.signal("-TERM");
    let status = harness.wait_exit();
    assert_eq!(status.code(), Some(0), "{status}");
    wait_gone(worker, "child of the persistent harness");
}

#[test]
fn new_session_and_resume_away_tear_down_the_live_child_and_keep_serving() {
    let fixture = fixture(false);
    let mut harness = Harness::start(&fixture, "alpha", false);
    let mut client = harness.connect();
    let first = client.spawn_worker(true);
    assert!(alive(first));

    // new_session: the child is settled before the roster is replaced.
    let ack = client.request(r#"{"type":"new_session","id":"n1"}"#, "n1");
    assert_eq!(ack["success"], true, "{ack}");
    assert!(
        !alive(first),
        "the child must be gone when new_session answers"
    );
    let roster = client.request(r#"{"type":"get_subagents","id":"g1"}"#, "g1");
    assert_eq!(
        roster["data"]["subagents"].as_array().map(Vec::len),
        Some(0),
        "{roster}"
    );
    assert!(
        harness.still_running(),
        "a session transition never exits the harness"
    );

    // The new session spawns its own child; resuming away settles it too.
    let second = client.spawn_worker(true);
    assert!(alive(second));
    let ack = client.request(
        r#"{"type":"resume_session","id":"r1","session":"alpha"}"#,
        "r1",
    );
    assert_eq!(ack["success"], true, "{ack}");
    assert!(
        !alive(second),
        "the child must be gone when resume_session answers"
    );
    let roster = client.request(r#"{"type":"get_subagents","id":"g2"}"#, "g2");
    assert_eq!(
        roster["data"]["subagents"].as_array().map(Vec::len),
        Some(0),
        "{roster}"
    );
    let state = client.request(r#"{"type":"get_state","id":"s2"}"#, "s2");
    assert_eq!(state["success"], true, "{state}");
    assert!(harness.still_running());
    drop(client);
    assert_eq!(harness.wait_exit().code(), Some(0));
}

/// A container-transport child (started detached by the create script,
/// reached through the proxy bridge) is settled by the session switch over
/// the same protocol, and its environment's retained kill argv runs once
/// through the compensation. The host never signals the container's pid.
#[test]
fn new_session_settles_a_container_child_and_runs_its_fake_kill_argv_once() {
    let fixture = fixture(true);
    let mut harness = Harness::start(&fixture, "container", false);
    let mut client = harness.connect();
    let _display_pid = client.spawn_worker(false);
    let child_pid: u32 = std::fs::read_to_string(fixture.base.join("child.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(alive(child_pid), "the detached container child runs");

    let ack = client.request(r#"{"type":"new_session","id":"n1"}"#, "n1");
    assert_eq!(ack["success"], true, "{ack}");
    wait_gone(child_pid, "container child");
    let killed = std::fs::read_to_string(fixture.base.join("killed.log"))
        .expect("the retained kill argv ran");
    assert_eq!(killed.lines().collect::<Vec<_>>(), vec!["killed env-1"]);
    let roster = client.request(r#"{"type":"get_subagents","id":"g1"}"#, "g1");
    assert_eq!(
        roster["data"]["subagents"].as_array().map(Vec::len),
        Some(0),
        "{roster}"
    );
    assert!(harness.still_running());
    drop(client);
    assert_eq!(harness.wait_exit().code(), Some(0));
    let killed = std::fs::read_to_string(fixture.base.join("killed.log")).unwrap();
    assert_eq!(killed.lines().count(), 1, "the kill argv runs exactly once");
}
