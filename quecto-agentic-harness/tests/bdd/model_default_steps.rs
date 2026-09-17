//! Steps for `model_default.feature` (#2024 S2): a production
//! `quecto agent --mode uds` started in a repository whose trusted overlay
//! pins `agents.defaults.model`/`effort`, a sibling repository without one,
//! and `set_model`/`set_effort` with `persist` writing one layer through
//! the safe configuration writer.

use super::*;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// One production agent process with a connected control socket.
#[derive(Debug)]
pub struct ModelDefaultProcess {
    child: Child,
    stream: UnixStream,
    reader: BufReader<UnixStream>,
    stderr_path: PathBuf,
    /// The last `set_model` / `set_effort` reply.
    last_reply: Option<serde_json::Value>,
    started: u32,
}

impl Drop for ModelDefaultProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn cwd(world: &QuectoWorld) -> PathBuf {
    world
        .cli_context
        .cwd
        .clone()
        .expect("BDD world should pin a hermetic cwd")
}

fn sibling_dir(world: &QuectoWorld) -> PathBuf {
    cwd(world)
        .parent()
        .expect("the cwd has a parent")
        .join("sibling")
}

fn global_config(world: &QuectoWorld) -> PathBuf {
    base_path(world).join("config.json")
}

fn file_json(path: &Path) -> serde_json::Value {
    let content =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&content).unwrap_or_else(|e| panic!("{} is not JSON: {e}", path.display()))
}

#[given(expr = "the global config file sets {string} to {string}")]
fn given_global_sets(world: &mut QuectoWorld, key_path: String, value: String) {
    let path = global_config(world);
    let mut document = file_json(&path);
    let mut current = &mut document;
    let segments: Vec<&str> = key_path.split('.').collect();
    let (last, parents) = segments.split_last().unwrap();
    for segment in parents {
        current = current
            .as_object_mut()
            .expect("an object on the way")
            .entry(*segment)
            .or_insert_with(|| serde_json::json!({}));
    }
    current
        .as_object_mut()
        .unwrap()
        .insert((*last).to_string(), serde_json::Value::String(value));
    std::fs::write(&path, serde_json::to_string_pretty(&document).unwrap()).expect("write global");
}

#[given(expr = "a sibling repository directory {string}")]
fn given_sibling(world: &mut QuectoWorld, name: String) {
    assert_eq!(name, "sibling", "the sibling directory is fixed by name");
    std::fs::create_dir_all(sibling_dir(world)).expect("create sibling");
}

fn start_agent(world: &mut QuectoWorld, directory: PathBuf) {
    // A previous agent of the scenario is torn down first so its socket
    // and session are not what the next one attaches to.
    let started = world
        .model_default_process
        .take()
        .map(|process| process.started + 1)
        .unwrap_or(0);
    std::fs::create_dir_all(&directory).expect("create the agent's directory");
    let base = base_path(world);
    let socket = base.join(format!("model-default-{started}.sock"));
    let stderr_path = base.join(format!("model-default-{started}.stderr"));
    let stderr = std::fs::File::create(&stderr_path).expect("create stderr capture");
    let mut child = Command::new(env!("CARGO_BIN_EXE_quecto"))
        .env("QUECTO_BASE_DIR", &base)
        .current_dir(&directory)
        .args(["agent", "--mode", "uds", "--no-session", "--socket"])
        .arg(&socket)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr)
        .spawn()
        .expect("spawn quecto agent");
    let deadline = Instant::now() + Duration::from_secs(30);
    let stream = loop {
        if let Ok(stream) = UnixStream::connect(&socket) {
            break stream;
        }
        if let Some(status) = child.try_wait().expect("agent status") {
            panic!(
                "the agent exited before its socket was ready: {status}\n{}",
                std::fs::read_to_string(&stderr_path).unwrap_or_default()
            );
        }
        assert!(
            Instant::now() < deadline,
            "socket startup timed out\n{}",
            std::fs::read_to_string(&stderr_path).unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    let reader = BufReader::new(stream.try_clone().expect("clone stream"));
    world.model_default_process = Some(ModelDefaultProcess {
        child,
        stream,
        reader,
        stderr_path,
        last_reply: None,
        started,
    });
}

#[when("a production UDS agent is started in the current directory")]
fn when_start_in_cwd(world: &mut QuectoWorld) {
    let directory = cwd(world);
    start_agent(world, directory);
}

#[when("a production UDS agent is started in the sibling directory")]
fn when_start_in_sibling(world: &mut QuectoWorld) {
    let directory = sibling_dir(world);
    start_agent(world, directory);
}

/// Send `command` and return the reply correlated by its id.
fn exchange(world: &mut QuectoWorld, mut command: serde_json::Value) -> serde_json::Value {
    let process = world
        .model_default_process
        .as_mut()
        .expect("a production agent is running");
    let id = format!(
        "md-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    command["id"] = serde_json::Value::String(id.clone());
    writeln!(process.stream, "{command}").expect("write command");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let mut line = String::new();
        match process.reader.read_line(&mut line) {
            Ok(0) => panic!(
                "the agent closed the socket\n{}",
                std::fs::read_to_string(&process.stderr_path).unwrap_or_default()
            ),
            Ok(_) => {
                let value: serde_json::Value =
                    serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line}"));
                if value["id"] == id {
                    return value;
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) => panic!("socket read failed: {error}"),
        }
        assert!(
            Instant::now() < deadline,
            "no reply to {command}\n{}",
            std::fs::read_to_string(&process.stderr_path).unwrap_or_default()
        );
    }
}

fn persist_field(persist: &str) -> Option<serde_json::Value> {
    match persist {
        "none" => None,
        other => Some(serde_json::Value::String(other.to_string())),
    }
}

fn send_persisting(world: &mut QuectoWorld, mut command: serde_json::Value, persist: &str) {
    config_discovery_steps::snapshot_config_files(world);
    if let Some(persist) = persist_field(persist) {
        command["persist"] = persist;
    }
    let reply = exchange(world, command);
    world
        .model_default_process
        .as_mut()
        .expect("a production agent is running")
        .last_reply = Some(reply);
}

#[when(expr = "the production agent is sent set_model {string} with persist {string}")]
fn when_set_model_persist(world: &mut QuectoWorld, model: String, persist: String) {
    send_persisting(
        world,
        serde_json::json!({"type": "set_model", "model": model}),
        &persist,
    );
}

#[when(
    expr = "the production agent is sent set_model provider {string} modelId {string} with persist {string}"
)]
fn when_set_model_provider_persist(
    world: &mut QuectoWorld,
    provider: String,
    model_id: String,
    persist: String,
) {
    send_persisting(
        world,
        serde_json::json!({"type": "set_model", "provider": provider, "modelId": model_id}),
        &persist,
    );
}

#[when(expr = "the production agent is sent set_effort {string} with persist {string}")]
fn when_set_effort_persist(world: &mut QuectoWorld, effort: String, persist: String) {
    send_persisting(
        world,
        serde_json::json!({"type": "set_effort", "effort": effort}),
        &persist,
    );
}

fn last_reply(world: &QuectoWorld) -> &serde_json::Value {
    world
        .model_default_process
        .as_ref()
        .expect("a production agent is running")
        .last_reply
        .as_ref()
        .expect("a set_model or set_effort was sent")
}

#[then("the production agent's reply should succeed")]
fn then_reply_ok(world: &mut QuectoWorld) {
    let reply = last_reply(world);
    assert_eq!(reply["success"], true, "{reply}");
}

#[then("the production agent's reply should fail")]
fn then_reply_failed(world: &mut QuectoWorld) {
    let reply = last_reply(world);
    assert_eq!(reply["success"], false, "{reply}");
}

#[then(expr = "the production agent's reply error should contain {string}")]
fn then_reply_error_contains(world: &mut QuectoWorld, needle: String) {
    let reply = last_reply(world);
    let error = reply["error"].as_str().unwrap_or_default();
    assert!(error.contains(&needle), "expected {needle:?} in {reply}");
}

#[then(
    expr = "the production agent's reply should report persisted scope {string} at the current directory's {string}"
)]
fn then_persisted_local(world: &mut QuectoWorld, scope: String, name: String) {
    let expected = cwd(world).join(name);
    let reply = last_reply(world);
    assert_eq!(reply["data"]["persisted"]["scope"], scope, "{reply}");
    assert_eq!(
        reply["data"]["persisted"]["path"],
        expected.to_string_lossy().as_ref(),
        "{reply}"
    );
}

#[then(
    expr = "the production agent's reply should report persisted scope {string} at the global {string}"
)]
fn then_persisted_global(world: &mut QuectoWorld, scope: String, name: String) {
    let expected = base_path(world).join(name);
    let reply = last_reply(world);
    assert_eq!(reply["data"]["persisted"]["scope"], scope, "{reply}");
    assert_eq!(
        reply["data"]["persisted"]["path"],
        expected.to_string_lossy().as_ref(),
        "{reply}"
    );
}

#[then("the production agent's reply should report nothing persisted")]
fn then_nothing_persisted(world: &mut QuectoWorld) {
    let reply = last_reply(world);
    assert!(
        reply["data"].get("persisted").is_none(),
        "unexpected persisted field: {reply}"
    );
}

fn state(world: &mut QuectoWorld) -> serde_json::Value {
    let reply = exchange(world, serde_json::json!({"type": "get_state"}));
    assert_eq!(reply["success"], true, "{reply}");
    reply["data"].clone()
}

#[then(expr = "the production agent's state should report model {string}")]
fn then_state_model(world: &mut QuectoWorld, expected: String) {
    let state = state(world);
    assert_eq!(state["model"], expected, "{state}");
}

#[then(expr = "the production agent's state should report effort {string}")]
fn then_state_effort(world: &mut QuectoWorld, expected: String) {
    let state = state(world);
    assert_eq!(state["effort"], expected, "{state}");
}
