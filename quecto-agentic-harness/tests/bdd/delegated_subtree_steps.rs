//! BDD steps for #1940: a real root → child → grandchild tree of `quecto`
//! processes, ended by acknowledged delegation (operator kill of the child)
//! or by the launch-bound parent-loss binding (SIGKILL of the child). The
//! root records no pid for the grandchild and never signals it; the test
//! observes the grandchild's process through `/proc` itself.
use std::path::PathBuf;
use std::time::{Duration, Instant};

use cucumber::{given, then, when};
use quecto::domain::tool::Tool;
use quecto::infrastructure::tools::agent_cmd::AgentCmdTool;
use quecto::infrastructure::tools::spawn::SpawnTool;
use quecto::infrastructure::tools::subagent_monitor_merge::REPORTED_DESCENDANT_PID;
use quecto::infrastructure::tools::subagent_registry::{SubagentRegistry, SubagentStatus};

use crate::QuectoWorld;

const READY_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Default)]
pub(crate) struct DelegatedSubtreeState {
    runtime: Option<tokio::runtime::Runtime>,
    registry: Option<SubagentRegistry>,
    spawn: Option<SpawnTool>,
    kill: Option<std::sync::Arc<dyn quecto::domain::tool::Tool>>,
    config_path: Option<PathBuf>,
    /// Registry key and pid of every observed row, by display label.
    observed: std::collections::HashMap<String, (String, u32)>,
    kill_result: Option<serde_json::Value>,
}

impl std::fmt::Debug for DelegatedSubtreeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DelegatedSubtreeState")
    }
}

fn state(world: &mut QuectoWorld) -> &mut DelegatedSubtreeState {
    &mut world.delegated_subtree
}

fn alive(pid: u32) -> bool {
    // SAFETY: signal 0 only probes existence of the given pid.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

/// The pid of the `quecto` process serving `quecto-agent-<key>.sock`: a
/// test-side `/proc` scan. The root's row for a grandchild carries no pid.
fn pid_serving_row(key: &str) -> Option<u32> {
    let needle = format!("quecto-agent-{key}.sock");
    std::fs::read_dir("/proc")
        .ok()?
        .flatten()
        .find_map(|entry| {
            let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
            let cmdline = std::fs::read(entry.path().join("cmdline")).ok()?;
            cmdline
                .split(|b| *b == 0)
                .any(|arg| arg.ends_with(needle.as_bytes()))
                .then_some(pid)
        })
}

fn child_binary() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap();
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                workspace_root.join(path)
            }
        })
        .unwrap_or_else(|| workspace_root.join("target"));
    let binary = target_dir.join("debug").join("quecto");
    assert!(
        binary.exists(),
        "build the quecto binary before this scenario: cargo build -p quecto-agentic-harness --bin quecto"
    );
    binary
}

/// A provider that makes the child launched with the `SPAWN_ONE` task spawn
/// exactly one grandchild ("grand") and then finish; every other turn just
/// answers.
async fn mount_provider(server: &wiremock::MockServer) {
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
                let arguments =
                    serde_json::json!({"agent_id": "grand", "task": "wait", "read_only": true})
                        .to_string();
                serde_json::json!({"tool_calls": [{
                    "index": 0, "id": "call-grand", "type": "function",
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
        .mount(server)
        .await;
}

#[given("a root harness whose \"SPAWN_ONE\" task makes a real child spawn a real grandchild")]
fn given_root(world: &mut QuectoWorld) {
    let dir = tempfile::tempdir().expect("temp dir");
    let base = dir.path().to_path_buf();
    world._temp_dir = Some(dir);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();
    let server = runtime.block_on(async {
        let server = wiremock::MockServer::start().await;
        mount_provider(&server).await;
        server
    });
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let config = serde_json::json!({
        "providers": {"openai": {"api_key": "sk-test", "api_base": server.uri()}},
        "agents": {"defaults": {"model": "openai-api/gpt-4o-mini", "workspace": workspace}}
    });
    let config_path = base.join("config.json");
    std::fs::write(&config_path, serde_json::to_string_pretty(&config).unwrap()).unwrap();
    std::mem::forget(server);
    let sockets = base.join("sockets");
    std::fs::create_dir_all(&sockets).unwrap();
    // SAFETY: @serial scenario; set before any child is launched, inherited by the whole tree.
    unsafe {
        std::env::set_var("QUECTO_CHILD_BINARY", child_binary());
        std::env::set_var("QUECTO_BASE_DIR", &base);
    }
    let registry = AgentCmdTool::new_registry();
    let (broadcast_tx, _rx) = tokio::sync::broadcast::channel::<String>(256);
    let spawn = SpawnTool::with_base_dir(vec![], base.clone())
        .with_socket_dir(sockets)
        .with_registry(registry.clone())
        .with_parent_config_path(Some(config_path.clone()))
        .with_event_forwarding(Some(broadcast_tx.clone()), Some("root".into()));
    let kill =
        crate::agent_cmd_tool_steps::termination_owners(&registry, Some(broadcast_tx)).kill_tool;
    world.agent_cmd_registry = Some(registry.clone());
    let s = state(world);
    s.runtime = Some(runtime);
    s.registry = Some(registry);
    s.spawn = Some(spawn);
    s.kill = Some(kill);
    s.config_path = Some(config_path);
}

#[when(expr = "the root spawns child {string} with task {string}")]
fn when_spawn(world: &mut QuectoWorld, child: String, task: String) {
    let s = state(world);
    let args = serde_json::json!({
        "agent_id": child, "task": task, "config": s.config_path.clone().unwrap(),
    })
    .to_string();
    let spawn = s.spawn.as_ref().unwrap();
    let result = s.runtime.as_ref().unwrap().block_on(async {
        tokio::time::timeout(READY_TIMEOUT, spawn.execute(&args))
            .await
            .expect("spawn bounded")
            .expect("spawn tool ran")
    });
    assert!(!result.is_error, "spawn failed: {}", result.content);
}

fn observe(world: &mut QuectoWorld, display: &str) -> (String, u32) {
    let s = state(world);
    let registry = s.registry.clone().unwrap();
    let started = Instant::now();
    loop {
        let found = {
            let entries = registry.lock().unwrap();
            entries
                .iter()
                .find(|(key, entry)| {
                    entry.effective_display_name(key) == display
                        && entry.status != SubagentStatus::Exited
                })
                .map(|(key, entry)| (key.clone(), entry.launch_generation.is_some(), entry.pid))
        };
        if let Some((key, launched, pid)) = found {
            let pid = if launched {
                Some(pid)
            } else {
                assert_eq!(pid, REPORTED_DESCENDANT_PID, "a merged row carries no pid");
                pid_serving_row(&key)
            };
            if let Some(pid) = pid.filter(|pid| *pid != 0) {
                s.observed.insert(display.to_string(), (key.clone(), pid));
                return (key, pid);
            }
        }
        assert!(
            started.elapsed() < READY_TIMEOUT,
            "{display} never appeared as a live row with an observable process"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[then(expr = "child {string} reports grandchild {string} with a launch generation and no pid")]
fn then_grandchild_reported(world: &mut QuectoWorld, child: String, grandchild: String) {
    let (child_key, child_pid) = observe(world, &child);
    let (grand_key, grand_pid) = observe(world, &grandchild);
    assert!(alive(child_pid) && alive(grand_pid));
    let registry = state(world).registry.clone().unwrap();
    let entries = registry.lock().unwrap();
    let grand = &entries[&grand_key];
    assert!(
        grand.launch_generation.is_none(),
        "the grandchild is not ours"
    );
    assert!(
        grand.delegated_identity().is_some(),
        "routable by generation"
    );
    assert_eq!(grand.parent_id.as_deref(), Some(child_key.as_str()));
    assert_eq!(grand.pid, REPORTED_DESCENDANT_PID);
    assert!(grand.owned_child.is_none());
    assert!(entries[&child_key].holds_owned_child());
}

#[when(expr = "the root kills {string} through its composed owner")]
fn when_kill(world: &mut QuectoWorld, child: String) {
    let s = state(world);
    let kill = s.kill.clone().unwrap();
    let args = serde_json::json!({"agent_id": child, "command": "kill"}).to_string();
    let result = s.runtime.as_ref().unwrap().block_on(async {
        tokio::time::timeout(Duration::from_secs(60), kill.execute(&args))
            .await
            .expect("kill bounded")
            .unwrap()
    });
    assert!(!result.is_error, "kill failed: {}", result.content);
    s.kill_result = Some(serde_json::from_str(&result.content).unwrap());
}

#[then(expr = "the kill of {string} answered {string}")]
fn then_kill_result(world: &mut QuectoWorld, child: String, result: String) {
    let s = state(world);
    let body = s.kill_result.clone().expect("a kill result");
    assert_eq!(body["result"], result, "{body}");
    assert_eq!(body["target"], s.observed[&child].0, "{body}");
}

#[when(expr = "the process of child {string} is SIGKILLed behind the root's back")]
fn when_sigkill(world: &mut QuectoWorld, child: String) {
    let pid = state(world).observed[&child].1;
    // SAFETY: the pid is this scenario's own launched child, observed live moments ago.
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
}

#[then(
    expr = "the processes of {string} and its grandchild {string} are gone within {int} seconds"
)]
fn then_gone(world: &mut QuectoWorld, child: String, grandchild: String, seconds: u64) {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    for name in [child, grandchild] {
        let pid = state(world).observed[&name].1;
        while alive(pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        if alive(pid) {
            // SAFETY: bounded cleanup of the scenario's own process.
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
            panic!("{name} ({pid}) did not exit within {seconds}s");
        }
    }
}

#[then(expr = "the grandchild {string} exited gracefully leaving no socket")]
fn then_graceful(world: &mut QuectoWorld, grandchild: String) {
    let key = state(world).observed[&grandchild].0.clone();
    let needle = format!("quecto-agent-{key}.sock");
    let sockets = std::env::var("QUECTO_BASE_DIR").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let mut leftover = Vec::new();
        for entry in walkdir(&PathBuf::from(&sockets)) {
            if entry.to_string_lossy().ends_with(&needle) {
                leftover.push(entry);
            }
        }
        if leftover.is_empty() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "a SIGKILLed or unreached grandchild leaves its socket behind: {leftover:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn walkdir(dir: &PathBuf) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walkdir(&path));
            } else {
                out.push(path);
            }
        }
    }
    out
}

#[then(expr = "every root row for {string} and {string} is exited")]
fn then_rows_exited(world: &mut QuectoWorld, child: String, grandchild: String) {
    let s = state(world);
    let registry = s.registry.clone().unwrap();
    let keys = [
        s.observed[&child].0.clone(),
        s.observed[&grandchild].0.clone(),
    ];
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let exited = {
            let entries = registry.lock().unwrap();
            keys.iter().all(|key| {
                entries
                    .get(key)
                    .is_some_and(|entry| entry.status == SubagentStatus::Exited)
            })
        };
        if exited {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "rows {keys:?} never reached exited"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}
