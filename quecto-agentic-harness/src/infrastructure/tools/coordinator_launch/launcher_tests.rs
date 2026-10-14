//! S4c: the launcher asks spawn for a coordinator in a new container,
//! within spawn's label bound and the session's policy and guards, reads
//! what spawn launched from its structured answer, and stops a coordinator
//! that came up outside a container.
use std::sync::Mutex;

use super::super::spawn_launch_ports::spawned_metadata;
use super::*;
use crate::application::tools::ports::ToolGuard;
use crate::domain::error::DomainError;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::vocabulary::Effort;
use crate::domain::tool::ToolDefinition;
use crate::infrastructure::tools::registry::ToolRegistryImpl;

fn request(task: &str, run: u32) -> CoordinatorRequest {
    CoordinatorRequest {
        task: Slug::parse(task).unwrap(),
        run,
        plan_version: 3,
        brief: "the brief".into(),
        model: Some("anthropic/claude-opus".into()),
        effort: Some(Effort::High),
    }
}

#[test]
fn a_coordinator_is_spawned_in_a_new_container_with_its_role() {
    let request = request("board-store", 2);
    assert_eq!(
        spawn_arguments(&request),
        json!({"agent_id": "board-board-store-run-2", "task": "the brief", "container": true,
               "coordinator": true, "model": "anthropic/claude-opus", "effort": "high"})
    );
    let unnamed = CoordinatorRequest {
        model: None,
        effort: None,
        ..request
    };
    let arguments = spawn_arguments(&unnamed);
    assert!(arguments.get("model").is_none() && arguments.get("effort").is_none());
}

/// A task id at the board's 64-byte limit still labels a valid agent_id,
/// at spawn's 64-character limit, and two such ids sharing their first
/// part get different labels.
#[test]
fn a_long_task_id_labels_a_valid_agent_id() {
    let long = "a".repeat(64);
    let twin = format!("{}b", "a".repeat(63));
    for run in [1, 999] {
        let label = label(&long, run);
        assert_eq!(label.chars().count(), MAX_AGENT_ID_CHARS, "{label}");
        assert!(validate_agent_id_format(&label).is_ok(), "{label}");
        assert!(label.starts_with("board-aaaa") && label.ends_with(&format!("-run-{run}")));
        assert_ne!(label, super::label(&twin, run), "a hash of the whole id");
    }
    let fits = "a".repeat(50);
    assert_eq!(label(&fits, 999), format!("board-{fits}-run-999"));
    assert_eq!(
        label(&fits, 999).chars().count(),
        64,
        "the longest kept whole"
    );
    assert_ne!(
        label(&"a".repeat(51), 999),
        format!("board-{}-run-999", "a".repeat(51))
    );
}

/// A tool that answers as told and keeps the arguments it was called with.
struct Answering {
    name: &'static str,
    answer: Mutex<Option<Result<ToolResult, DomainError>>>,
    called: Mutex<Vec<Value>>,
    stall: bool,
}

impl Answering {
    fn new(name: &'static str, answer: Result<ToolResult, DomainError>) -> Arc<Self> {
        Arc::new(Self {
            name,
            answer: Mutex::new(Some(answer)),
            called: Mutex::default(),
            stall: false,
        })
    }

    fn stalling(name: &'static str) -> Arc<Self> {
        Arc::new(Self {
            name,
            answer: Mutex::new(None),
            called: Mutex::default(),
            stall: true,
        })
    }

    fn called(&self) -> Vec<Value> {
        self.called.lock().unwrap().clone()
    }
}

impl Tool for Answering {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name.into(),
            description: "a test tool".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ToolResult, DomainError>> + Send + '_>,
    > {
        self.called
            .lock()
            .unwrap()
            .push(serde_json::from_str(arguments).unwrap());
        let answer = self.answer.lock().unwrap().take();
        let stall = self.stall;
        Box::pin(async move {
            if stall {
                std::future::pending::<()>().await;
            }
            answer.unwrap_or_else(|| Err(DomainError::Tool("asked twice".into())))
        })
    }
}

fn answered(content: &str, metadata: Option<String>) -> Result<ToolResult, DomainError> {
    Ok(ToolResult {
        content: content.into(),
        is_error: false,
        image_blocks: vec![],
        delivery_metadata: metadata,
    })
}

/// A permit open, with `guards`, as a registry with a spawn tool sets it.
fn open_permit(guards: Vec<Arc<dyn ToolGuard>>) -> SpawnPermit {
    let permit = SpawnPermit::default();
    let mut registry = ToolRegistryImpl::new();
    registry.register(Answering::new("spawn", answered("", None)));
    for guard in guards {
        registry.register_guard(guard);
    }
    registry.watch_spawn_permit(permit.clone());
    assert!(permit.allowed());
    permit
}

/// A launcher over `spawn` and `agent_cmd`, under `permit`.
fn launcher(
    spawn: &Arc<Answering>,
    kill: &Arc<Answering>,
    permit: SpawnPermit,
) -> SpawnCoordinatorLauncher {
    SpawnCoordinatorLauncher::new(
        Arc::clone(spawn) as Arc<dyn Tool>,
        Arc::clone(kill) as Arc<dyn Tool>,
        permit,
    )
}

/// Runs `launch` on a blocking thread of a runtime, as a board call does.
fn on_a_board_thread<T: Send + 'static>(launch: impl FnOnce() -> T + Send + 'static) -> T {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let launching = runtime.spawn_blocking(launch);
    runtime.block_on(launching).unwrap()
}

#[test]
fn a_launch_reads_what_spawn_launched_from_its_structured_answer() {
    let metadata = spawned_metadata("7f3a-uuid", Some("C12"));
    // The words are no source: only the metadata names the coordinator.
    let spawn = Answering::new("spawn", answered("Subagent 'x' is running", Some(metadata)));
    let kill = Answering::new("agent_cmd", answered("", None));
    let launching = launcher(&spawn, &kill, open_permit(vec![]));
    let launched = on_a_board_thread(move || launching.launch(&request("a", 1)));
    assert_eq!(
        launched,
        Ok(LaunchedCoordinator {
            agent: "7f3a-uuid".into(),
            environment_ref: "C12".into(),
        })
    );
    assert_eq!(spawn.called()[0]["agent_id"], json!("board-a-run-1"));
    assert!(kill.called().is_empty(), "nothing stopped");
}

/// A coordinator that came up outside a container is stopped before the
/// launch is refused; one whose stop fails says so.
#[test]
fn a_coordinator_outside_a_container_is_stopped_before_the_refusal() {
    let local = || Some(spawned_metadata("7f3a-uuid", None));
    for (kill_answer, said) in [
        (answered("killed", None), "it was stopped"),
        (
            Err(DomainError::Tool("no such agent".into())),
            "stopping it failed: ",
        ),
    ] {
        let spawn = Answering::new("spawn", answered("running", local()));
        let kill = Answering::new("agent_cmd", kill_answer);
        let launching = launcher(&spawn, &kill, open_permit(vec![]));
        let refused = on_a_board_thread(move || launching.launch(&request("a", 1)));
        let reason = refused.unwrap_err().0;
        assert!(
            reason.starts_with("the coordinator came up outside a container, so ")
                && reason.contains(said),
            "{reason}"
        );
        assert_eq!(
            kill.called(),
            vec![json!({"agent_id": "7f3a-uuid", "command": "kill"})]
        );
    }
}

/// An answer that names no child, a refused spawn and a failed one are
/// refusals in their own words; nothing is stopped.
#[test]
fn a_spawn_that_launched_nothing_is_refused_in_its_words() {
    let error_result = Ok(ToolResult {
        content: "Error: no container runtime".into(),
        is_error: true,
        image_blocks: vec![],
        delivery_metadata: None,
    });
    for (answer, reason) in [
        (
            answered("odd", Some("{}".into())),
            "spawn answered no running coordinator: odd",
        ),
        (
            answered("odd", None),
            "spawn answered no running coordinator: odd",
        ),
        (error_result, "Error: no container runtime"),
        (
            Err(DomainError::Tool("broken".into())),
            "tool error: broken",
        ),
    ] {
        let spawn = Answering::new("spawn", answer);
        let kill = Answering::new("agent_cmd", answered("", None));
        let launching = launcher(&spawn, &kill, open_permit(vec![]));
        let refused = on_a_board_thread(move || launching.launch(&request("a", 1)));
        assert_eq!(refused, Err(LaunchRefused(reason.into())));
        assert!(kill.called().is_empty());
    }
}

/// A guard that refuses every spawn.
struct NoSpawn;

impl ToolGuard for NoSpawn {
    fn check(&self, tool_name: &str, arguments: &str) -> Result<(), String> {
        assert_eq!(tool_name, "spawn");
        assert!(
            arguments.contains("board-a-run-1"),
            "the very call is judged"
        );
        Err("spawn is guarded".into())
    }
}

/// A closed permit and a guard's refusal stop the launch before spawn is
/// asked; so does a launch with no runtime to wait on.
#[test]
fn a_launch_the_session_could_not_make_never_asks_spawn() {
    let spawn = Answering::new("spawn", answered("", None));
    let kill = Answering::new("agent_cmd", answered("", None));
    let closed = launcher(&spawn, &kill, SpawnPermit::default());
    assert!(!closed.may_launch());
    assert_eq!(
        closed.launch(&request("a", 1)),
        Err(LaunchRefused(NOT_ALLOWED.into()))
    );
    let guarded = launcher(&spawn, &kill, open_permit(vec![Arc::new(NoSpawn)]));
    assert!(guarded.may_launch());
    assert_eq!(
        guarded.launch(&request("a", 1)),
        Err(LaunchRefused("spawn is guarded".into()))
    );
    let unhosted = launcher(&spawn, &kill, open_permit(vec![]));
    assert_eq!(
        unhosted.launch(&request("a", 1)),
        Err(LaunchRefused(NO_RUNTIME.into())),
        "no runtime here"
    );
    assert!(spawn.called().is_empty(), "spawn never asked");
}

/// A spawn that never answers is given up on at the launcher's timeout.
#[test]
fn a_spawn_that_never_answers_times_out() {
    let spawn = Answering::stalling("spawn");
    let kill = Answering::new("agent_cmd", answered("", None));
    let launching =
        launcher(&spawn, &kill, open_permit(vec![])).with_timeout(Duration::from_millis(50));
    let refused = on_a_board_thread(move || launching.launch(&request("a", 1)));
    assert_eq!(
        refused,
        Err(LaunchRefused("spawn did not answer within 50ms".into()))
    );
    assert_eq!(LAUNCH_TIMEOUT, Duration::from_secs(900));
}
