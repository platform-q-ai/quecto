//! Fixture for the operator-selected termination steps (#1936, #1882): a
//! root registry with real owned processes, fake direct-child endpoints with
//! scripted behaviours, the composed `agent_cmd kill` owner over the
//! production adapters, and the helpers the steps drive it with.
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use quecto::application::subagents::dto::{
    CompensateFailedLaunchRequest, FailedLaunchCompensated, ObservedExit,
};
use quecto::domain::ids::AgentUuid;
use quecto::domain::subagent_teardown::LaunchGeneration;
use quecto::domain::tool::{Tool, ToolResult};
use quecto::infrastructure::extensions::native::KillToolWiring;
use quecto::infrastructure::processes::owned_child_supervisor::{
    ChildHandleId, OwnedChildSupervisor, ProcessGroup,
};
use quecto::infrastructure::tools::subagent_registry::{
    SubagentEntry, SubagentRegistry, TeardownPhase, new_exit_signal_channel, new_registry,
};
use quecto::infrastructure::tools::subagent_teardown_wiring::{
    SubagentLifecycleUseCases, build_lifecycle_use_cases,
};

use crate::QuectoWorld;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Behaviour {
    Acknowledge,
    Refuse,
    Malformed,
    Unreachable,
}

/// One fake direct-child endpoint: answers per `behaviour`, records every
/// request, and — when `exit_pid` is set — ends the owned process itself
/// on a `shutdown` it acknowledged (the child exiting on its own).
pub(crate) struct Endpoint {
    pub(crate) path: PathBuf,
    pub(crate) requests: Arc<Mutex<Vec<serde_json::Value>>>,
}

pub(crate) fn serve(
    runtime: &tokio::runtime::Runtime,
    behaviour: Behaviour,
    exit_pid: Arc<AtomicU64>,
) -> Endpoint {
    let path = std::env::temp_dir().join(format!("q-st-{}.sock", uuid::Uuid::new_v4().simple()));
    let _ = std::fs::remove_file(&path);
    let requests = Arc::new(Mutex::new(Vec::new()));
    if behaviour == Behaviour::Unreachable {
        return Endpoint { path, requests };
    }
    let listener = {
        let _guard = runtime.enter();
        tokio::net::UnixListener::bind(&path).unwrap()
    };
    let seen = requests.clone();
    runtime.spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let seen = seen.clone();
            let exit_pid = exit_pid.clone();
            tokio::spawn(async move {
                use tokio::io::AsyncWriteExt;
                let (read, mut write) = tokio::io::split(stream);
                let mut reader = tokio::io::BufReader::new(read);
                let Ok(Some(incoming)) =
                    quecto_line_io::read_frame_or_legacy_line(&mut reader, 64 * 1024).await
                else {
                    return;
                };
                let bytes = match incoming {
                    quecto_line_io::Incoming::Frame(b)
                    | quecto_line_io::Incoming::LegacyLine(b) => b,
                };
                let request: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                seen.lock().unwrap().push(request.clone());
                let reply = match behaviour {
                    Behaviour::Acknowledge => serde_json::json!({
                        "type": "response", "id": request["id"], "command": request["type"],
                        "success": true,
                    })
                    .to_string(),
                    Behaviour::Refuse => serde_json::json!({
                        "type": "response", "id": request["id"], "command": request["type"],
                        "success": false, "error": "refused",
                    })
                    .to_string(),
                    Behaviour::Malformed => "{not json".to_string(),
                    Behaviour::Unreachable => unreachable!(),
                };
                let _ = write.write_all(format!("{reply}\n").as_bytes()).await;
                if behaviour == Behaviour::Acknowledge && request["type"] == "shutdown" {
                    let pid = exit_pid.load(Ordering::SeqCst);
                    if pid != 0 {
                        // The child ends itself after acknowledging.
                        // SAFETY: the pid is the fixture's own sleeping process.
                        unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
                    }
                }
                let _ = quecto_line_io::read_frame_or_legacy_line(&mut reader, 1024).await;
            });
        }
    });
    Endpoint { path, requests }
}

#[derive(Default)]
pub(crate) struct SelectedTerminationState {
    pub(crate) runtime: Option<tokio::runtime::Runtime>,
    pub(crate) registry: Option<SubagentRegistry>,
    pub(crate) supervisor: Option<Arc<OwnedChildSupervisor>>,
    pub(crate) broadcast_rx: Option<tokio::sync::broadcast::Receiver<String>>,
    pub(crate) broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    pub(crate) endpoints: Vec<(String, Endpoint)>,
    pub(crate) handles: Vec<(String, ChildHandleId)>,
    pub(crate) kill: Option<Arc<dyn Tool>>,
    pub(crate) lifecycle: Option<SubagentLifecycleUseCases>,
    pub(crate) result: Option<ToolResult>,
    pub(crate) observed: Option<ObservedExit>,
    pub(crate) rollbacks: Vec<FailedLaunchCompensated>,
    pub(crate) generation: u64,
}

impl Drop for SelectedTerminationState {
    /// Every fixture process this scenario still holds is ended through its
    /// owner, so no `sleep` outlives the scenario.
    fn drop(&mut self) {
        let Some(supervisor) = self.supervisor.clone() else {
            return;
        };
        for (_, handle) in self.handles.drain(..) {
            supervisor.request_termination(
                handle,
                Box::pin(async {
                    quecto::infrastructure::processes::owned_child_supervisor::ProtocolOutcome::Negative(
                        "scenario over".into(),
                    )
                }),
                quecto::infrastructure::processes::owned_child_supervisor::TerminationBudget {
                    exit_after_ack: Duration::from_millis(100),
                    term_grace: Duration::from_millis(500),
                    kill_grace: Duration::from_secs(2),
                },
            );
        }
        for (_, endpoint) in self.endpoints.drain(..) {
            let _ = std::fs::remove_file(&endpoint.path);
        }
    }
}

impl std::fmt::Debug for SelectedTerminationState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SelectedTerminationState")
    }
}

pub(crate) fn state(world: &mut QuectoWorld) -> &mut SelectedTerminationState {
    let s = &mut world.selected_termination;
    if s.runtime.is_none() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let (broadcast_tx, broadcast_rx) = tokio::sync::broadcast::channel::<String>(64);
        let registry = new_registry();
        s.kill = Some(quecto::composition::subagent_termination::build_kill_tool(
            KillToolWiring {
                owner: AgentUuid::new("root"),
                registry: registry.clone(),
                broadcast_tx: Some(broadcast_tx.clone()),
                notify_tx: None,
            },
        ));
        s.lifecycle = Some(build_lifecycle_use_cases(
            registry.clone(),
            Some(broadcast_tx.clone()),
            None,
        ));
        s.registry = Some(registry);
        s.broadcast_rx = Some(broadcast_rx);
        s.broadcast_tx = Some(broadcast_tx);
        s.supervisor = Some(Arc::new(OwnedChildSupervisor::new()));
        s.runtime = Some(runtime);
    }
    s
}

pub(crate) fn behaviour_of(text: &str) -> Behaviour {
    match text {
        "acknowledges commands" => Behaviour::Acknowledge,
        "refuses commands" => Behaviour::Refuse,
        "answers with a malformed acknowledgement" => Behaviour::Malformed,
        "is unreachable" => Behaviour::Unreachable,
        other => panic!("unknown endpoint behaviour {other:?}"),
    }
}

pub(crate) fn label(uuid: &str) -> String {
    match uuid {
        "A" => "alpha".into(),
        "D" => "delta".into(),
        "E" => "echo".into(),
        other => other.to_lowercase(),
    }
}

pub(crate) fn spawn_process(
    s: &mut SelectedTerminationState,
    argv: &[&str],
) -> (ChildHandleId, u32) {
    let supervisor = s.supervisor.clone().unwrap();
    let runtime = s.runtime.as_ref().unwrap();
    let mut command = tokio::process::Command::new(argv[0]);
    command
        .args(&argv[1..])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let spawned = runtime
        .block_on(supervisor.spawn(command, ProcessGroup::Inherited))
        .expect("spawn fixture process");
    (spawned.handle, spawned.display_pid.0)
}

#[derive(Clone, Copy)]
pub(crate) enum Process {
    None,
    Sleeping,
    ShortLived,
    ExitsWhenTold,
}

pub(crate) fn add_launched(
    world: &mut QuectoWorld,
    uuid: &str,
    behaviour: Behaviour,
    process: Process,
) {
    let s = state(world);
    let exit_pid = Arc::new(AtomicU64::new(0));
    let endpoint = serve(s.runtime.as_ref().unwrap(), behaviour, exit_pid.clone());
    s.generation += 1;
    let mut entry =
        SubagentEntry::with_identity(AgentUuid::new(uuid), label(uuid), endpoint.path.clone(), 0);
    entry.launch_generation = Some(LaunchGeneration::new(s.generation));
    entry.parent_id = Some("root".into());
    let (exit_tx, _rx) = new_exit_signal_channel();
    entry.exit_signal_tx = Some(exit_tx);
    let told = matches!(process, Process::ExitsWhenTold);
    let spawned = match process {
        Process::None => None,
        Process::Sleeping | Process::ExitsWhenTold => Some(spawn_process(s, &["sleep", "300"])),
        Process::ShortLived => Some(spawn_process(s, &["sleep", "1"])),
    };
    if let Some((handle, pid)) = spawned {
        entry.owned_child = Some(handle);
        entry.owned_child_supervisor = s.supervisor.clone();
        entry.pid = pid;
        s.handles.push((uuid.to_owned(), handle));
        if told {
            exit_pid.store(u64::from(pid), Ordering::SeqCst);
        }
    }
    s.registry
        .as_ref()
        .unwrap()
        .lock()
        .unwrap()
        .insert(uuid.to_owned(), entry);
    s.endpoints.push((uuid.to_owned(), endpoint));
}

pub(crate) fn add_reported(world: &mut QuectoWorld, uuid: &str, parent: &str, generation: u64) {
    let s = state(world);
    let mut entry =
        SubagentEntry::with_identity(AgentUuid::new(uuid), label(uuid), PathBuf::new(), 4242);
    entry.reported_generation = Some(LaunchGeneration::new(generation));
    entry.parent_id = Some(parent.into());
    s.registry
        .as_ref()
        .unwrap()
        .lock()
        .unwrap()
        .insert(uuid.to_owned(), entry);
}

pub(crate) fn wait_compensated(s: &mut SelectedTerminationState, uuid: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let phase = s.registry.as_ref().unwrap().lock().unwrap()[uuid].teardown_phase();
        if phase == TeardownPhase::Compensated {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{uuid} was never compensated (phase {phase:?})"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub(crate) fn rollback(world: &mut QuectoWorld, uuid: &str) {
    let s = state(world);
    let use_case = s.lifecycle.clone().unwrap().compensate_launch;
    let child = s.registry.as_ref().unwrap().lock().unwrap()[uuid]
        .delegated_identity()
        .unwrap();
    let outcome = s.runtime.as_ref().unwrap().block_on(async move {
        tokio::time::timeout(
            Duration::from_secs(30),
            use_case.execute(CompensateFailedLaunchRequest {
                child,
                owns_environment: false,
            }),
        )
        .await
        .expect("rollback settles within the bound")
    });
    s.rollbacks.push(outcome);
}

pub(crate) fn body(world: &mut QuectoWorld) -> serde_json::Value {
    let s = state(world);
    let result = s.result.as_ref().expect("a kill ran");
    serde_json::from_str(&result.content)
        .unwrap_or_else(|_| panic!("kill result is not JSON: {}", result.content))
}

pub(crate) fn broadcasts(world: &mut QuectoWorld) -> Vec<serde_json::Value> {
    let s = state(world);
    let rx = s.broadcast_rx.as_mut().unwrap();
    let mut events = Vec::new();
    while let Ok(line) = rx.try_recv() {
        let value: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        if value["type"] == "subagent_state_changed" {
            events.push(value);
        }
    }
    events
}

pub(crate) fn listed(event: &serde_json::Value) -> Vec<String> {
    let mut ids: Vec<String> = event["subagents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["agentUuid"].as_str().unwrap().to_owned())
        .collect();
    ids.sort();
    ids
}
