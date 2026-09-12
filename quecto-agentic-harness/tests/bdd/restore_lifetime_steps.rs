//! BDD steps for launcher lifetime and session restore without child
//! readoption (#1937): the pure lifetime policy, a real launcher-created
//! child running without `--persist`, and a real in-process restoring
//! harness resuming a session written by an earlier harness whose legacy
//! roster rows name sockets that must never be connected to.
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use cucumber::{given, then, when};
use quecto::composition::subagent_teardown::build_teardown_graph;
use quecto::domain::harness_lifetime::{HarnessLifetime, HarnessLifetimeError};
use quecto::domain::message::Message;
use quecto::domain::session::{Session, SessionStore};
use quecto::domain::tool::Tool;
use quecto::infrastructure::persistence::session_store::FileSessionStore;
use quecto::infrastructure::tools::agent_cmd::{AgentCmdTool, SubagentRegistry};
use quecto::infrastructure::tools::spawn::SpawnTool;
use quecto::infrastructure::tools::subagent_registry::SubagentEntry;
use quecto::interface::cli::uds::{UdsLoopArgs, run_uds_loop};

use crate::QuectoWorld;

#[derive(Default)]
pub(crate) struct RestoreLifetimeState {
    lifetime: Option<Result<HarnessLifetime, HarnessLifetimeError>>,
    harness: Option<RestoringHarness>,
    /// Sockets named by legacy roster rows; a probe would be accepted here.
    silent_listeners: Vec<(String, UnixListener)>,
    legacy_uuids: Vec<String>,
    legacy_session_key: Option<String>,
    /// The re-spawned child as registered, captured before any session
    /// switch removes its row.
    respawned: Option<SubagentEntry>,
}

impl std::fmt::Debug for RestoreLifetimeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RestoreLifetimeState")
    }
}

/// A top-level in-process harness (default lifetime: exits when its last
/// client disconnects) with a subagent registry the scenario can inspect.
struct RestoringHarness {
    socket_path: PathBuf,
    handle: Option<std::thread::JoinHandle<i32>>,
    client: Option<UnixStream>,
    registry: SubagentRegistry,
    resume_ack: Option<serde_json::Value>,
}

fn state(world: &mut QuectoWorld) -> &mut RestoreLifetimeState {
    &mut world.restore_lifetime
}

fn base(world: &QuectoWorld) -> PathBuf {
    world.cli_context.base_dir.clone().expect("temp base dir")
}

// ── Lifetime policy ──────────────────────────────────────────────────────────

#[when(expr = "the harness lifetime is resolved with persist {string} and launched {string}")]
fn when_lifetime_resolved(world: &mut QuectoWorld, persist: String, launched: String) {
    let flag = |raw: &str| match raw {
        "true" => true,
        "false" => false,
        other => panic!("not a bool: {other}"),
    };
    state(world).lifetime = Some(HarnessLifetime::resolve(flag(&persist), flag(&launched)));
}

#[then(expr = "the resolved lifetime is {string}")]
fn then_lifetime_is(world: &mut QuectoWorld, expected: String) {
    let resolved = state(world).lifetime.expect("a resolution");
    match expected.as_str() {
        "until_last_client_disconnects" => {
            let lifetime = resolved.unwrap();
            assert_eq!(lifetime, HarnessLifetime::UntilLastClientDisconnects);
            assert!(lifetime.exits_when_last_client_disconnects());
            assert!(!lifetime.is_launch_bound());
        }
        "persistent" => {
            let lifetime = resolved.unwrap();
            assert_eq!(lifetime, HarnessLifetime::Persistent);
            assert!(!lifetime.exits_when_last_client_disconnects());
            assert!(!lifetime.is_launch_bound());
        }
        "launch_bound" => {
            let lifetime = resolved.unwrap();
            assert_eq!(lifetime, HarnessLifetime::LaunchBound);
            assert!(!lifetime.exits_when_last_client_disconnects());
            assert!(lifetime.is_launch_bound());
        }
        "refused" => {
            let error = resolved.unwrap_err();
            assert_eq!(error, HarnessLifetimeError::LaunchedChildCannotPersist);
            assert!(error.to_string().contains("--persist is refused"));
        }
        other => panic!("unknown lifetime {other}"),
    }
}

// ── A real launcher-created child ────────────────────────────────────────────

fn live_entry(world: &QuectoWorld, agent_id: &str) -> SubagentEntry {
    let registry = world.agent_cmd_registry.as_ref().expect("registry");
    let entries = registry.lock().unwrap();
    entries
        .values()
        .find(|entry| entry.display_name == agent_id)
        .unwrap_or_else(|| panic!("no live entry for {agent_id}"))
        .clone()
}

#[then(expr = "the child argv for {string} carries no --persist")]
fn then_argv_no_persist(world: &mut QuectoWorld, agent_id: String) {
    let entry = live_entry(world, &agent_id);
    let cmdline = std::fs::read(format!("/proc/{}/cmdline", entry.pid)).unwrap();
    let args: Vec<String> = cmdline
        .split(|b| *b == 0)
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect();
    assert!(
        !args.iter().any(|a| a == "--persist"),
        "a launcher-created child must not persist: {args:?}"
    );
    assert!(args.iter().any(|a| a == "--spawned"));
    assert!(args.iter().any(|a| a == "--parent-control"));
}

#[when(expr = "an ordinary probe connects to {string} and disconnects")]
fn when_probe_churns(world: &mut QuectoWorld, agent_id: String) {
    let entry = live_entry(world, &agent_id);
    let mut stream = UnixStream::connect(&entry.socket_path).expect("connect to child");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream.write_all(b"{\"type\":\"get_state\"}\n").unwrap();
    assert!(read_until(&mut stream, |v| v["command"] == "get_state").is_some());
    drop(stream);
    std::thread::sleep(Duration::from_millis(300));
}

#[then(expr = "the child process of {string} is still running")]
fn then_child_still_running(world: &mut QuectoWorld, agent_id: String) {
    let entry = live_entry(world, &agent_id);
    let exit_rx = entry
        .exit_signal_tx
        .as_ref()
        .expect("exit signal")
        .subscribe();
    assert!(
        exit_rx.borrow().is_none(),
        "client churn must not end a launch-bound child"
    );
    assert!(entry.holds_owned_child(), "the supervisor still retains it");
    assert!(entry.socket_path.exists());
}

// ── A restoring harness resuming a legacy session ────────────────────────────

fn legacy_row(
    id: &str,
    socket: &std::path::Path,
    liveness: &str,
    status: &str,
) -> serde_json::Value {
    serde_json::json!({
        "agentUuid": id,
        "displayName": format!("worker-{id}"),
        "sessionKey": id,
        "socketPath": socket,
        "pid": std::process::id(),
        "liveness": liveness,
        "restoreReason": "legacy_unspecified",
        "parentId": "parent",
        "readOnly": true,
        "status": status,
        "deliveredMessageOrdinal": 3,
        "pendingMessageReports": [{"receipt":"r1","response":"past child message","ordinal":4}]
    })
}

#[given(
    expr = "session {string} was saved by an earlier harness with legacy child rows naming sockets that must never be connected to"
)]
fn given_legacy_session(world: &mut QuectoWorld, session_name: String) {
    let base = base(world);
    let key = Session::build_key("cli", &session_name);
    // Let the store place the file, then overwrite it with the legacy shape.
    let store = FileSessionStore::new(&base);
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(store.save(&Session {
        key: key.clone(),
        messages: vec![Message::user("placeholder")],
        workflow_run: None,
        subagent_roster: Vec::new(),
    }))
    .unwrap();
    let sessions = base.join("sessions");
    let path = std::fs::read_dir(&sessions)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|ext| ext == "json"))
        .expect("the store wrote one session file");

    let sockets = base.join("legacy-sockets");
    std::fs::create_dir_all(&sockets).unwrap();
    let mut listeners = Vec::new();
    let mut rows = Vec::new();
    for (id, liveness, status) in [
        ("legacy-live", "live", "running"),
        ("legacy-detached", "detached", "idle"),
    ] {
        let socket = sockets.join(format!("{id}.sock"));
        let listener = UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        listeners.push((id.to_string(), listener));
        rows.push(legacy_row(id, &socket, liveness, status));
    }
    rows.push(legacy_row(
        "legacy-dead",
        &sockets.join("legacy-dead.sock"),
        "dead",
        "exited",
    ));
    rows.push(legacy_row(
        "legacy-gone",
        &sockets.join("legacy-gone.sock"),
        "live",
        "idle",
    ));
    rows.push(serde_json::json!({"displayName": "malformed"}));
    let snapshot = serde_json::json!({
        "type": "snapshot",
        "key": key,
        "messages": [
            {"role":"user","content":"persisted transcript survives restore"},
            {"role":"assistant","content":"persisted answer"},
            {"role":"assistant","content":"child worker-legacy-live reported: done"},
        ],
        "workflow_run": {"template_id": "feature", "done": [true, true, false], "active_issue": null},
        "subagent_roster": rows,
    });
    std::fs::write(&path, format!("{snapshot}\n")).unwrap();
    let s = state(world);
    s.silent_listeners = listeners;
    s.legacy_uuids = vec![
        "legacy-live".into(),
        "legacy-detached".into(),
        "legacy-dead".into(),
        "legacy-gone".into(),
    ];
    s.legacy_session_key = Some(key);
}

#[given("a restoring UDS harness with a subagent registry")]
fn given_restoring_harness(world: &mut QuectoWorld) {
    world.session_name = Some("restoring-master".into());
    world.no_session = false;
    world._workflow_enabled = true;
    let base = base(world);
    let ctx = crate::uds_steps::build_uds_agent(world, &base).expect("agent built");
    let socket_path = base.join("restoring.sock");
    let _ = std::fs::remove_file(&socket_path);
    let registry = AgentCmdTool::new_registry();
    let registry_for_loop = registry.clone();
    let base_dir = base.clone();
    let sp = socket_path.clone();
    let handle = std::thread::spawn(move || {
        let crate::uds_steps::UdsAgentContext {
            agent,
            model,
            session_key,
            ephemeral,
            ext_registry,
            persist: _,
            workflow_state,
            workflow_config,
            broadcast_tx,
            mut provider_reload,
            provider_reload_inputs,
        } = ctx;
        run_uds_loop(UdsLoopArgs {
            agent,
            base_dir: &base_dir,
            workspace: &base_dir,
            session_key,
            model,
            ephemeral,
            system_prompt: String::new(),
            socket_path: sp,
            socket_override: None,
            session_store_override: None,
            ext_registry: Some(ext_registry),
            // Top-level default: the scenario's client keeps it alive and
            // its disconnect ends it.
            lifetime: HarnessLifetime::UntilLastClientDisconnects,
            notification_rx: None,
            subagent_registry: Some(registry_for_loop),
            workflow_state,
            workflow_config,
            broadcast_tx,
            provider_reload: Some(&mut provider_reload),
            provider_reload_inputs: Some(&provider_reload_inputs),
            parent_control: None,
            teardown_graph: Some(build_teardown_graph),
        })
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    while !socket_path.exists() {
        assert!(Instant::now() < deadline, "restoring harness never bound");
        std::thread::sleep(Duration::from_millis(10));
    }
    let client = UnixStream::connect(&socket_path).expect("connect");
    client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    // The registry is what agent_cmd would target from inside the harness.
    world.agent_cmd_registry = Some(registry.clone());
    world.agent_cmd_tool = Some(AgentCmdTool::new(registry.clone()));
    state(world).harness = Some(RestoringHarness {
        socket_path,
        handle: Some(handle),
        client: Some(client),
        registry,
        resume_ack: None,
    });
}

#[given("a stale operational row is registered in the restoring harness")]
fn given_stale_row(world: &mut QuectoWorld) {
    let base = base(world);
    let harness = state(world).harness.as_ref().unwrap();
    harness.registry.lock().unwrap().insert(
        "stale".into(),
        SubagentEntry::new(base.join("stale.sock"), 0),
    );
}

/// Read newline-delimited JSON until one matches, or EOF/timeout.
fn read_until(
    stream: &mut UnixStream,
    want: impl Fn(&serde_json::Value) -> bool,
) -> Option<serde_json::Value> {
    let mut buffer = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match stream.read(&mut byte) {
            Ok(0) | Err(_) => return None,
            Ok(_) => {}
        }
        if byte[0] == b'\n' {
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&buffer)
                && want(&value)
            {
                return Some(value);
            }
            buffer.clear();
        } else {
            buffer.push(byte[0]);
        }
    }
}

fn send(world: &mut QuectoWorld, line: &str, id: &str) -> serde_json::Value {
    let harness = state(world).harness.as_mut().unwrap();
    let client = harness.client.as_mut().expect("client connected");
    client.write_all(format!("{line}\n").as_bytes()).unwrap();
    read_until(client, |v| v["id"] == id).unwrap_or_else(|| panic!("no response for {id}"))
}

#[when(expr = "the client resumes session {string}")]
fn when_client_resumes(world: &mut QuectoWorld, session_name: String) {
    let ack = send(
        world,
        &format!(
            "{{\"type\":\"resume_session\",\"id\":\"resume-1\",\"session\":\"{session_name}\"}}"
        ),
        "resume-1",
    );
    state(world).harness.as_mut().unwrap().resume_ack = Some(ack);
}

#[then(expr = "the resume succeeds and restores {int} messages including the past child message")]
fn then_resume_succeeds(world: &mut QuectoWorld, count: u64) {
    let ack = state(world)
        .harness
        .as_ref()
        .unwrap()
        .resume_ack
        .clone()
        .expect("resume ack");
    assert_eq!(ack["success"], true, "{ack}");
    assert_eq!(ack["data"]["messageCount"], count, "{ack}");
    let messages = send(
        world,
        "{\"type\":\"get_messages\",\"id\":\"msgs-1\"}",
        "msgs-1",
    );
    let contents: Vec<String> = messages["data"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["content"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(
        contents
            .iter()
            .any(|c| c == "child worker-legacy-live reported: done"),
        "past child message must survive: {contents:?}"
    );
    assert!(contents.iter().any(|c| c == "persisted answer"));
}

#[then(expr = "the restored workflow run is {string} with {int} completed steps")]
fn then_workflow_restored(world: &mut QuectoWorld, template: String, done: u64) {
    let state_response = send(
        world,
        "{\"type\":\"get_state\",\"id\":\"state-1\"}",
        "state-1",
    );
    let workflow = &state_response["data"]["workflow"];
    assert_eq!(
        workflow["activeTemplate"]["id"], template,
        "{state_response}"
    );
    // The slim projection reports the current (first undone) step 1-based:
    // `done` completed steps put it at index done + 1.
    assert_eq!(
        workflow["currentStep"]["index"],
        done + 1,
        "{state_response}"
    );
    assert_eq!(workflow["currentStep"]["done"], false, "{state_response}");
}

#[then("no persisted child socket was connected to")]
fn then_no_socket_connected(world: &mut QuectoWorld) {
    for (id, listener) in &state(world).silent_listeners {
        match listener.accept() {
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            other => panic!("restore probed the persisted socket of {id}: {other:?}"),
        }
    }
}

#[then("the operational roster of the restoring harness is empty")]
fn then_roster_empty(world: &mut QuectoWorld) {
    let harness = state(world).harness.as_ref().unwrap();
    let entries = harness.registry.lock().unwrap();
    assert!(
        entries.is_empty(),
        "no operational child row after restore: {:?}",
        entries.keys().collect::<Vec<_>>()
    );
}

#[then("get_subagents from the restoring harness lists no children")]
fn then_get_subagents_empty(world: &mut QuectoWorld) {
    let response = send(
        world,
        "{\"type\":\"get_subagents\",\"id\":\"subs-1\"}",
        "subs-1",
    );
    assert_eq!(response["success"], true, "{response}");
    let subagents = response["data"]["subagents"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(subagents.is_empty(), "{response}");
}

#[then("no legacy child row is targetable through agent_cmd")]
fn then_legacy_not_targetable(world: &mut QuectoWorld) {
    let registry = state(world).harness.as_ref().unwrap().registry.clone();
    let uuids = state(world).legacy_uuids.clone();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let tool = AgentCmdTool::new(registry);
    for uuid in uuids {
        for target in [uuid.clone(), format!("worker-{uuid}")] {
            for command in ["get_state", "kill"] {
                let result = rt
                    .block_on(tool.execute(&format!(
                        r#"{{"agent_id":"{target}","command":"{command}"}}"#
                    )))
                    .unwrap();
                assert!(result.is_error, "{target} {command} must not resolve");
            }
            let send = rt
                .block_on(tool.execute(&format!(
                    r#"{{"agent_id":"{target}","command":"send","message":"hi"}}"#
                )))
                .unwrap();
            assert!(send.is_error, "{target} must not be sendable");
        }
    }
}

#[when("the client starts a new session")]
fn when_new_session(world: &mut QuectoWorld) {
    let ack = send(
        world,
        "{\"type\":\"new_session\",\"id\":\"new-1\"}",
        "new-1",
    );
    assert_eq!(ack["success"], true, "{ack}");
}

#[then(
    expr = "the saved session {string} keeps its {int} messages and writes no child pid or socket"
)]
fn then_saved_session_migrated(world: &mut QuectoWorld, session_name: String, count: usize) {
    let base = base(world);
    let key = Session::build_key("cli", &session_name);
    let store = FileSessionStore::new(&base);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let saved = rt
        .block_on(store.load(&key))
        .unwrap()
        .expect("session kept");
    assert_eq!(saved.messages.len(), count);
    let raw = std::fs::read_dir(base.join("sessions"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| std::fs::read_to_string(e.path()).unwrap_or_default())
        .collect::<String>();
    // The legacy snapshot line was rewritten by the save on the way out: no
    // child pid or socket survives in any session file.
    assert!(
        !raw.contains("legacy-sockets"),
        "socket path persisted: {raw}"
    );
    assert!(!raw.contains("\"pid\""), "pid persisted: {raw}");
}

#[when("the client disconnects from the restoring harness")]
fn when_client_disconnects(world: &mut QuectoWorld) {
    drop(state(world).harness.as_mut().unwrap().client.take());
}

#[then(expr = "the restoring harness exits within {int} seconds")]
fn then_restoring_harness_exits(world: &mut QuectoWorld, seconds: u64) {
    let harness = state(world).harness.as_mut().unwrap();
    let handle = harness.handle.take().unwrap();
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while !handle.is_finished() {
        assert!(
            Instant::now() < deadline,
            "the harness did not exit in time"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(handle.join().unwrap(), 0);
    assert!(!harness.socket_path.exists());
}

#[when(expr = "the restoring harness re-spawns subagent {string} with initial task {string}")]
fn when_harness_respawns(world: &mut QuectoWorld, agent_id: String, task: String) {
    let base = base(world);
    let config_path = world.config_path.clone().expect("config path");
    let registry = state(world).harness.as_ref().unwrap().registry.clone();
    let socket_dir = base.join("sockets");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let tool = SpawnTool::with_base_dir(vec![], base)
        .with_socket_dir(socket_dir)
        .with_registry(registry.clone());
    let args = serde_json::json!({
        "agent_id": agent_id,
        "task": task,
        "config": config_path,
        "read_only": true
    });
    let rt = tokio::runtime::Runtime::new().unwrap();
    world.spawn_result = Some(rt.block_on(tool.execute(&args.to_string())).unwrap());
    state(world).respawned = registry
        .lock()
        .unwrap()
        .values()
        .find(|entry| entry.display_name == agent_id)
        .cloned();
    world.agent_cmd_registry = Some(registry);
    // The runtime carries the monitor task: the child's bound parent
    // connection lives as long as the scenario.
    world.spawn_runtimes.push(rt);
}

#[then(expr = "the re-spawned {string} has a fresh identity unlike every legacy row")]
fn then_fresh_identity(world: &mut QuectoWorld, agent_id: String) {
    let entry = live_entry(world, &agent_id);
    let legacy = state(world).legacy_uuids.clone();
    assert!(!legacy.iter().any(|uuid| uuid == entry.agent_uuid.as_str()));
    assert!(entry.launch_generation.is_some());
    assert!(entry.pid > 0);
    let entries = state(world)
        .harness
        .as_ref()
        .unwrap()
        .registry
        .lock()
        .unwrap();
    assert_eq!(
        entries.len(),
        1,
        "only the re-spawned worker is operational"
    );
    assert!(!entries.keys().any(|key| legacy.contains(key)));
}

fn process_alive(pid: u32) -> bool {
    // Nothing is delivered with signal 0.
    // SAFETY: signal 0 only probes the existence of the pid this scenario spawned.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

/// The session switch tore the re-spawned child down: the supervisor no
/// longer retains its handle, its process is gone and its socket removed.
#[then(expr = "the re-spawned {string} is gone within {int} seconds")]
fn then_respawned_gone(world: &mut QuectoWorld, agent_id: String, seconds: u64) {
    let entry = state(world)
        .respawned
        .clone()
        .expect("re-spawned entry captured");
    assert_eq!(entry.display_name, agent_id);
    let supervisor = entry.owned_child_supervisor.clone().expect("supervisor");
    let handle = entry.owned_child.expect("owned handle");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime
        .block_on(async {
            tokio::time::timeout(Duration::from_secs(seconds), supervisor.wait_exit(handle)).await
        })
        .expect("the child must exit within the bound");
    assert!(!supervisor.retains(handle));
    let deadline = Instant::now() + Duration::from_secs(5);
    while process_alive(entry.pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!process_alive(entry.pid), "the child process must be gone");
    while entry.socket_path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        !entry.socket_path.exists(),
        "a graceful exit removes the socket"
    );
}
