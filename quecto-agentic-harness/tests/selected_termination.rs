//! Real-process selected termination (#1936, #1882): a root harness (this
//! test, running the production `SpawnTool` and the composed `agent_cmd
//! kill` owner) launches a real `quecto` child A whose mock provider makes
//! it launch two children of its own, B and C. Killing B ends B's subtree
//! while A and C survive — the root never sends A self shutdown, it sends the
//! authenticated `terminate_delegated_agent` command and A ends B over its
//! own edge. Killing A afterwards ends A and, through A's own teardown, C.
//!
//! Every wait is bounded so a regression fails instead of hanging.
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use quecto::domain::tool::Tool;
use quecto::infrastructure::extensions::native::KillToolWiring;
use quecto::infrastructure::tools::subagent_registry::{SubagentRegistry, SubagentStatus};

const READY_TIMEOUT: Duration = Duration::from_secs(120);
const EXIT_BOUND: Duration = Duration::from_secs(30);

fn alive(pid: u32) -> bool {
    // SAFETY: signal 0 only probes existence of the given pid.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

/// A provider that makes the child launched with the `SPAWN_TWO` task spawn
/// two children (B then C) and then finish; every other child just answers.
/// `final_delay` holds the spawner's final completion open, so the child
/// can be killed while it is busy awaiting its provider.
/// The returned counter counts the spawner's delayed (final) completions
/// requested so far: once it is 1 the spawner is busy awaiting one.
async fn provider(final_delay: Duration) -> (wiremock::MockServer, Arc<AtomicUsize>) {
    let server = wiremock::MockServer::start().await;
    let delayed_calls = Arc::new(AtomicUsize::new(0));
    let counter = delayed_calls.clone();
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(move |request: &wiremock::Request| {
            let input: serde_json::Value = request.body_json().unwrap();
            let messages = input["messages"].as_array().cloned().unwrap_or_default();
            let is_spawner = messages.iter().any(|m| {
                m["role"] == "user"
                    && m["content"]
                        .as_str()
                        .is_some_and(|content| content.contains("SPAWN_TWO"))
            });
            let tool_results = messages.iter().filter(|m| m["role"] == "tool").count();
            let delta = if is_spawner && tool_results < 2 {
                let name = if tool_results == 0 { "bee" } else { "cee" };
                let arguments =
                    serde_json::json!({"agent_id": name, "task": "wait", "read_only": true})
                        .to_string();
                serde_json::json!({"tool_calls": [{
                    "index": 0,
                    "id": format!("call-{name}"),
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
            let delayed = is_spawner && finish == "stop";
            if delayed {
                counter.fetch_add(1, Ordering::SeqCst);
            }
            let chunk = serde_json::json!({
                "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
                "usage": {"prompt_tokens": 10, "completion_tokens": 3, "total_tokens": 13}
            });
            let response = wiremock::ResponseTemplate::new(200).set_body_raw(
                format!("data: {chunk}\n\ndata: [DONE]\n\n"),
                "text/event-stream",
            );
            if delayed {
                response.set_delay(final_delay)
            } else {
                response
            }
        })
        .mount(&server)
        .await;
    (server, delayed_calls)
}

fn write_config(base: &Path, api_base: &str) -> PathBuf {
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let config = serde_json::json!({
        "providers": {"openai": {"api_key": "sk-test", "api_base": api_base}},
        "agents": {"defaults": {"model": "openai-api/gpt-4o-mini", "workspace": workspace}}
    });
    let path = base.join("config.json");
    std::fs::write(&path, serde_json::to_string_pretty(&config).unwrap()).unwrap();
    path
}

struct Row {
    key: String,
    pid: u32,
}

fn row(registry: &SubagentRegistry, display: &str) -> Option<Row> {
    let entries = registry.lock().unwrap();
    entries
        .iter()
        .find(|(key, entry)| {
            entry.effective_display_name(key) == display
                && entry.status != SubagentStatus::Exited
                && entry.pid != 0
        })
        .map(|(key, entry)| Row {
            key: key.clone(),
            pid: entry.pid,
        })
}

async fn wait_for_rows(registry: &SubagentRegistry, displays: &[&str]) -> Vec<Row> {
    let started = Instant::now();
    loop {
        let rows: Vec<Row> = displays
            .iter()
            .filter_map(|display| row(registry, display))
            .collect();
        if rows.len() == displays.len() {
            return rows;
        }
        assert!(
            started.elapsed() < READY_TIMEOUT,
            "rows {displays:?} never appeared in the root registry"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_gone(pid: u32, what: &str) {
    let started = Instant::now();
    while alive(pid) {
        if started.elapsed() > EXIT_BOUND {
            // SAFETY: best-effort cleanup of the pid this test observed.
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
            panic!("{what} ({pid}) did not exit within the bound");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn killing_nested_b_ends_its_subtree_while_a_and_c_survive_then_killing_a_ends_c() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().to_path_buf();
    let (server, _) = provider(Duration::ZERO).await;
    let config = write_config(&base, &server.uri());
    let sockets = base.join("sockets");
    std::fs::create_dir_all(&sockets).unwrap();
    // SAFETY: set before any child is launched; children inherit it.
    unsafe {
        std::env::set_var("QUECTO_CHILD_BINARY", env!("CARGO_BIN_EXE_quecto"));
        std::env::set_var("QUECTO_BASE_DIR", &base);
    }
    let registry = quecto::infrastructure::tools::agent_cmd::AgentCmdTool::new_registry();
    let (broadcast_tx, _broadcast_rx) = tokio::sync::broadcast::channel::<String>(256);
    let spawn =
        quecto::infrastructure::tools::spawn::SpawnTool::with_base_dir(vec![], base.clone())
            .with_socket_dir(sockets.clone())
            .with_registry(registry.clone())
            .with_parent_config_path(Some(config.clone()))
            .with_event_forwarding(Some(broadcast_tx.clone()), Some("root".into()));
    let kill = quecto::composition::subagent_termination::build_kill_tool(KillToolWiring {
        owner: quecto::domain::ids::AgentUuid::new("root"),
        registry: registry.clone(),
        broadcast_tx: Some(broadcast_tx),
        notify_tx: None,
    });

    let args = serde_json::json!({
        "agent_id": "aye",
        "task": "SPAWN_TWO",
        "config": config,
    });
    let result = tokio::time::timeout(READY_TIMEOUT, spawn.execute(&args.to_string()))
        .await
        .expect("spawn bounded")
        .expect("spawn tool ran");
    assert!(!result.is_error, "spawn failed: {}", result.content);

    // A's own launches reach the root through A's reported snapshot, with
    // their launch generations, so they are addressable from here.
    let rows = wait_for_rows(&registry, &["aye", "bee", "cee"]).await;
    let (a, b, c) = (&rows[0], &rows[1], &rows[2]);
    assert!(alive(a.pid) && alive(b.pid) && alive(c.pid));
    {
        let entries = registry.lock().unwrap();
        assert!(entries[&b.key].delegated_identity().is_some());
        assert!(entries[&b.key].launch_generation.is_none(), "B is not ours");
        assert_eq!(entries[&b.key].parent_id.as_deref(), Some(a.key.as_str()));
    }

    // Kill nested B: routed through A, which stays alive, as does C.
    let killed = tokio::time::timeout(
        EXIT_BOUND,
        kill.execute(&serde_json::json!({"agent_id": "bee", "command": "kill"}).to_string()),
    )
    .await
    .expect("kill bounded")
    .unwrap();
    assert!(!killed.is_error, "kill of B failed: {}", killed.content);
    let body: serde_json::Value = serde_json::from_str(&killed.content).unwrap();
    assert_eq!(body["result"], "graceful", "{}", killed.content);
    assert_eq!(body["target"], b.key);
    wait_gone(b.pid, "B").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(alive(a.pid), "A must survive the kill of its child");
    assert!(alive(c.pid), "A's unrelated child must survive");
    {
        let entries = registry.lock().unwrap();
        assert_eq!(entries[&b.key].status, SubagentStatus::Exited);
        assert_ne!(entries[&a.key].status, SubagentStatus::Exited);
        assert_ne!(entries[&c.key].status, SubagentStatus::Exited);
    }
    // A second kill of B is refused with no effect.
    let again = kill
        .execute(&serde_json::json!({"agent_id": "bee", "command": "kill"}).to_string())
        .await
        .unwrap();
    assert!(again.is_error);
    assert!(
        again.content.contains("already exited") || again.content.contains("not found"),
        "{}",
        again.content
    );

    // Kill A directly: graceful over its own edge, and its subtree (C) ends
    // through A's teardown — never through a signal from here.
    let killed = tokio::time::timeout(
        EXIT_BOUND,
        kill.execute(&serde_json::json!({"agent_id": "aye", "command": "kill"}).to_string()),
    )
    .await
    .expect("kill bounded")
    .unwrap();
    assert!(!killed.is_error, "kill of A failed: {}", killed.content);
    let body: serde_json::Value = serde_json::from_str(&killed.content).unwrap();
    assert_eq!(body["result"], "graceful", "{}", killed.content);
    let removed: Vec<&str> = body["killed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(removed[0], a.key);
    assert!(removed.contains(&c.key.as_str()), "{removed:?}");
    wait_gone(a.pid, "A").await;
    wait_gone(c.pid, "C").await;
    let all_exited = registry
        .lock()
        .unwrap()
        .values()
        .all(|entry| entry.status == SubagentStatus::Exited);
    assert!(all_exited, "every row is exited");
    // A graceful exit removes the child's socket file.
    let a_socket = sockets.join(format!("quecto-agent-{}.sock", a.key));
    let deadline = Instant::now() + Duration::from_secs(5);
    while a_socket.exists() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        !a_socket.exists(),
        "A's socket must be removed by a graceful exit"
    );
}

/// A busy direct child — mid-turn, awaiting a delayed provider completion —
/// is killed gracefully: the shutdown cancels its turn, the loop observes
/// the exit readiness while the turn is running, and the child exits with
/// status 0 well inside the exit budget, its session persisted and its own
/// children compensated. A busy intermediate still forwards a nested kill
/// promptly.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn killing_a_busy_child_is_graceful_and_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().to_path_buf();
    // Longer than the whole termination budget: only a cancelled turn
    // can end the child within the bound asserted below.
    let (server, delayed_calls) = provider(Duration::from_secs(40)).await;
    let config = write_config(&base, &server.uri());
    let sockets = base.join("sockets");
    std::fs::create_dir_all(&sockets).unwrap();
    // SAFETY: set before any child is launched; children inherit it.
    unsafe {
        std::env::set_var("QUECTO_CHILD_BINARY", env!("CARGO_BIN_EXE_quecto"));
        std::env::set_var("QUECTO_BASE_DIR", &base);
    }
    let registry = quecto::infrastructure::tools::agent_cmd::AgentCmdTool::new_registry();
    let (broadcast_tx, _broadcast_rx) = tokio::sync::broadcast::channel::<String>(256);
    let spawn =
        quecto::infrastructure::tools::spawn::SpawnTool::with_base_dir(vec![], base.clone())
            .with_socket_dir(sockets.clone())
            .with_registry(registry.clone())
            .with_parent_config_path(Some(config.clone()))
            .with_event_forwarding(Some(broadcast_tx.clone()), Some("root".into()));
    let kill = quecto::composition::subagent_termination::build_kill_tool(KillToolWiring {
        owner: quecto::domain::ids::AgentUuid::new("root"),
        registry: registry.clone(),
        broadcast_tx: Some(broadcast_tx),
        notify_tx: None,
    });
    let args = serde_json::json!({"agent_id": "aye", "task": "SPAWN_TWO", "config": config});
    let result = tokio::time::timeout(READY_TIMEOUT, spawn.execute(&args.to_string()))
        .await
        .expect("spawn bounded")
        .expect("spawn tool ran");
    assert!(!result.is_error, "spawn failed: {}", result.content);
    let rows = wait_for_rows(&registry, &["aye", "bee", "cee"]).await;
    let (a, b, c) = (&rows[0], &rows[1], &rows[2]);
    // A is now busy: its final completion is held open by the provider.
    let busy_deadline = Instant::now() + READY_TIMEOUT;
    while delayed_calls.load(Ordering::SeqCst) == 0 {
        assert!(Instant::now() < busy_deadline, "A never became busy");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        registry.lock().unwrap()[&a.key].status,
        SubagentStatus::Running
    );
    // A busy intermediate forwards a nested kill promptly.
    let started = Instant::now();
    let killed = tokio::time::timeout(
        EXIT_BOUND,
        kill.execute(&serde_json::json!({"agent_id": "bee", "command": "kill"}).to_string()),
    )
    .await
    .expect("kill bounded")
    .unwrap();
    assert!(!killed.is_error, "kill of B failed: {}", killed.content);
    let body: serde_json::Value = serde_json::from_str(&killed.content).unwrap();
    assert_eq!(body["result"], "graceful", "{}", killed.content);
    wait_gone(b.pid, "B").await;
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "a busy intermediate must forward promptly, took {:?}",
        started.elapsed()
    );
    assert!(alive(a.pid) && alive(c.pid));
    // Kill busy A itself.
    let a_exit = registry.lock().unwrap()[&a.key]
        .exit_signal_tx
        .clone()
        .expect("A's exit signal");
    let started = Instant::now();
    let killed = tokio::time::timeout(
        EXIT_BOUND,
        kill.execute(&serde_json::json!({"agent_id": "aye", "command": "kill"}).to_string()),
    )
    .await
    .expect("kill bounded")
    .unwrap();
    let elapsed = started.elapsed();
    assert_eq!(
        delayed_calls.load(Ordering::SeqCst),
        1,
        "the shutdown started no further provider call"
    );
    assert!(!killed.is_error, "kill of A failed: {}", killed.content);
    let body: serde_json::Value = serde_json::from_str(&killed.content).unwrap();
    assert_eq!(body["result"], "graceful", "{}", killed.content);
    assert!(
        elapsed < Duration::from_secs(3),
        "a busy child must be cancelled and exit promptly, took {elapsed:?}"
    );
    wait_gone(a.pid, "A").await;
    wait_gone(c.pid, "C").await;
    let exit = a_exit.borrow().clone().expect("A's exit was reaped");
    assert_eq!(
        (exit.exit_code, exit.signal),
        (Some(0), None),
        "graceful exit status"
    );
    let sessions = base.join("sessions");
    let persisted = std::fs::read_dir(&sessions)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .any(|entry| entry.file_name().to_string_lossy().contains(&a.key))
        })
        .unwrap_or(false);
    assert!(
        persisted,
        "A's session must be persisted on a graceful exit"
    );
    let entries = registry.lock().unwrap();
    for key in [&a.key, &b.key, &c.key] {
        assert_eq!(entries[key].status, SubagentStatus::Exited, "{key}");
    }
}
