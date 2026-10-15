//! S4c: the launcher asks spawn for a coordinator in a new container,
//! within spawn's label bound and the session's policy and guards, reads
//! what spawn launched from its structured answer, and stops a coordinator
//! that came up outside a container.
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use super::super::spawn_launch_ports::SpawnedChild;
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
    registry.observe_policy(std::sync::Arc::new(permit.clone()));
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
        super::super::subagent_registry::new_registry(),
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
    let metadata = SpawnedChild {
        uuid: "7f3a-uuid".into(),
        environment_ref: Some("C12".into()),
    }
    .metadata();
    // The words are no source: only the metadata names the coordinator.
    let spawn = Answering::new("spawn", answered("Subagent 'x' is running", Some(metadata)));
    let kill = Answering::new("agent_cmd", answered("", None));
    let launching = launcher(&spawn, &kill, open_permit(vec![]));
    let launched =
        on_a_board_thread(move || launching.launch(&request("a", 1), &AtomicBool::new(false)));
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
/// launch is refused; one whose stop failed or timed out is named as
/// still running, so the start undoes nothing under it.
#[test]
fn a_coordinator_outside_a_container_is_stopped_before_the_refusal() {
    let local = || {
        Some(
            SpawnedChild {
                uuid: "7f3a-uuid".into(),
                environment_ref: None,
            }
            .metadata(),
        )
    };
    let why = "the coordinator came up outside a container, so ";
    let error_result = Ok(ToolResult {
        content: "Error: no such agent".into(),
        is_error: true,
        image_blocks: vec![],
        delivery_metadata: None,
    });
    for (kill, want) in [
        (
            Answering::new("agent_cmd", answered("killed", None)),
            LaunchRefused::because(format!("{why}it was stopped")),
        ),
        (
            Answering::new("agent_cmd", Err(DomainError::Tool("no such agent".into()))),
            LaunchRefused::leaving(
                format!("{why}stopping it failed: tool error: no such agent"),
                "7f3a-uuid",
            ),
        ),
        (
            Answering::new("agent_cmd", error_result),
            LaunchRefused::leaving(
                format!("{why}stopping it failed: Error: no such agent"),
                "7f3a-uuid",
            ),
        ),
        (
            Answering::stalling("agent_cmd"),
            LaunchRefused::leaving(
                format!("{why}stopping it timed out (the stop goes on)"),
                "7f3a-uuid",
            ),
        ),
    ] {
        let spawn = Answering::new("spawn", answered("running", local()));
        let launching = launcher(&spawn, &kill, open_permit(vec![]))
            .with_timeouts(LAUNCH_TIMEOUT, Duration::from_millis(50));
        let refused =
            on_a_board_thread(move || launching.launch(&request("a", 1), &AtomicBool::new(false)));
        assert_eq!(refused, Err(want));
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
        let refused =
            on_a_board_thread(move || launching.launch(&request("a", 1), &AtomicBool::new(false)));
        assert_eq!(refused, Err(LaunchRefused::because(reason)));
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
        closed.launch(&request("a", 1), &AtomicBool::new(false)),
        Err(LaunchRefused::because(NOT_ALLOWED))
    );
    let guarded = launcher(&spawn, &kill, open_permit(vec![Arc::new(NoSpawn)]));
    assert!(guarded.may_launch());
    assert_eq!(
        guarded.launch(&request("a", 1), &AtomicBool::new(false)),
        Err(LaunchRefused::because("spawn is guarded"))
    );
    let unhosted = launcher(&spawn, &kill, open_permit(vec![]));
    assert_eq!(
        unhosted.launch(&request("a", 1), &AtomicBool::new(false)),
        Err(LaunchRefused::because(NO_RUNTIME)),
        "no runtime here"
    );
    assert!(spawn.called().is_empty(), "spawn never asked");
}

/// A spawn that never answers is given up on at the launcher's timeout.
#[test]
fn a_spawn_that_never_answers_times_out() {
    let spawn = Answering::stalling("spawn");
    let kill = Answering::new("agent_cmd", answered("", None));
    let launching = launcher(&spawn, &kill, open_permit(vec![]))
        .with_timeouts(Duration::from_millis(50), KILL_TIMEOUT);
    let refused =
        on_a_board_thread(move || launching.launch(&request("a", 1), &AtomicBool::new(false)));
    assert_eq!(
        refused,
        Err(LaunchRefused::because("spawn did not answer within 50ms"))
    );
    assert_eq!(LAUNCH_TIMEOUT, Duration::from_secs(900));
}

/// Review round 2: a running spawn that carries no structured answer has
/// whatever uuid its words name stopped, and the launch refused; the
/// stop has its own short bound.
#[test]
fn a_running_spawn_without_its_metadata_is_stopped_by_its_words() {
    let spawn = Answering::new(
        "spawn",
        answered(
            "Subagent 'x' is running (uuid=9a1c). Use agent_cmd to interact.",
            None,
        ),
    );
    let kill = Answering::new("agent_cmd", answered("killed", None));
    let launching = launcher(&spawn, &kill, open_permit(vec![]));
    let refused =
        on_a_board_thread(move || launching.launch(&request("a", 1), &AtomicBool::new(false)));
    assert_eq!(
        refused,
        Err(LaunchRefused::because(
            "spawn named no container for the coordinator, so it was stopped"
        ))
    );
    assert_eq!(
        kill.called(),
        vec![json!({"agent_id": "9a1c", "command": "kill"})]
    );
    assert_eq!(uuid_in("running (uuid=) now"), None);
    assert_eq!(uuid_in("no uuid here"), None);
    assert_eq!(KILL_TIMEOUT, Duration::from_secs(30));
}

/// Review round 3: the deadline has come at it, not only past it.
#[test]
fn a_deadline_has_come_at_it() {
    let now = tokio::time::Instant::now();
    assert!(past(now, now), "at the deadline");
    assert!(past(now, now + Duration::from_millis(1)));
    assert!(!past(now + Duration::from_millis(1), now));
}

/// Review rounds 3-4: a subagent runs while it is live or detached and in
/// each status of one that runs; a dead or exited one does not.
#[test]
fn a_coordinator_runs_while_it_is_live_and_running() {
    use crate::domain::ids::AgentUuid;
    use crate::domain::sessions::entities::session::SubagentLiveness;
    use crate::infrastructure::tools::subagent_registry::{SubagentEntry, SubagentStatus};
    let entry = |liveness, status| {
        let mut entry = SubagentEntry::with_identity(
            AgentUuid::mint(),
            "board-a-run-1".into(),
            "/tmp/coordinator.sock".into(),
            0,
        );
        entry.persisted_liveness = liveness;
        entry.status = status;
        entry
    };
    for status in [
        SubagentStatus::Starting,
        SubagentStatus::Idle,
        SubagentStatus::Running,
        SubagentStatus::Error,
    ] {
        assert!(
            runs(&entry(SubagentLiveness::Live, status.clone())),
            "{status:?}"
        );
        assert!(
            !runs(&entry(SubagentLiveness::Dead, status.clone())),
            "dead {status:?}"
        );
        assert!(
            runs(&entry(SubagentLiveness::Detached, status.clone())),
            "detached {status:?}"
        );
    }
    for liveness in [SubagentLiveness::Live, SubagentLiveness::Detached] {
        assert!(!runs(&entry(liveness, SubagentStatus::Exited)));
    }
}

/// An `agent_cmd` whose kill takes 300 ms, then notes that it finished.
struct SlowKill(Arc<std::sync::atomic::AtomicBool>);

impl Tool for SlowKill {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "agent_cmd".into(),
            description: "a slow kill".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }

    fn execute(
        &self,
        _: &str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ToolResult, DomainError>> + Send + '_>,
    > {
        let finished = Arc::clone(&self.0);
        Box::pin(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            finished.store(true, std::sync::atomic::Ordering::SeqCst);
            answered("killed", None)
        })
    }
}

/// Review (MED-3): a kill that outlasts the wait is not dropped half-way
/// (that would leave the child claimed as stopping, refusing every later
/// kill): it goes on to its end, while the launch already says the
/// coordinator may still run.
#[test]
fn a_kill_past_its_wait_goes_on_to_its_end() {
    let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let spawn = Answering::new(
        "spawn",
        answered(
            "running",
            Some(
                SpawnedChild {
                    uuid: "7f3a-uuid".into(),
                    environment_ref: None,
                }
                .metadata(),
            ),
        ),
    );
    let kill: Arc<dyn Tool> = Arc::new(SlowKill(Arc::clone(&finished)));
    let launching = SpawnCoordinatorLauncher::new(
        spawn as Arc<dyn Tool>,
        kill,
        open_permit(vec![]),
        super::super::subagent_registry::new_registry(),
    )
    .with_timeouts(LAUNCH_TIMEOUT, Duration::from_millis(50));
    let seen = Arc::clone(&finished);
    let (refused, finished_at_once, finished_later) = on_a_board_thread(move || {
        let refused = launching.launch(&request("a", 1), &AtomicBool::new(false));
        let at_once = seen.load(std::sync::atomic::Ordering::SeqCst);
        let began = std::time::Instant::now();
        while !seen.load(std::sync::atomic::Ordering::SeqCst)
            && began.elapsed() < Duration::from_secs(5)
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        (
            refused,
            at_once,
            seen.load(std::sync::atomic::Ordering::SeqCst),
        )
    });
    assert_eq!(
        refused,
        Err(LaunchRefused::leaving(
            "the coordinator came up outside a container, so stopping it timed out (the stop \
             goes on)",
            "7f3a-uuid",
        ))
    );
    assert!(!finished_at_once, "the launch did not wait for the kill");
    assert!(finished_later, "the kill went on to its end");
}

/// #2481 S4c follow-up: once its coordinator has stopped (and its box,
/// with its board, may be gone), the board the teardown kept under that
/// coordinator is how the launcher reports the run's swarm ended: for a
/// run it launched only, and only once a board was kept.
#[test]
fn a_launched_runs_swarm_is_reported_from_the_board_kept_under_its_coordinator() {
    use crate::application::environments::dto::SwarmBoardSnapshot;
    use crate::application::environments::ports::EndedSwarmBoards;
    use crate::application::project_board::fold_back::{SwarmEnd, SwarmItemState};
    let metadata = SpawnedChild {
        uuid: "7f3a-uuid".into(),
        environment_ref: Some("C12".into()),
    }
    .metadata();
    let spawn = Answering::new("spawn", answered("", Some(metadata)));
    let kill = Answering::new("agent_cmd", answered("", None));
    let kept = Arc::new(KeptSwarmBoards::default());
    let launching =
        Arc::new(launcher(&spawn, &kill, open_permit(vec![])).keeping(Arc::clone(&kept)));
    let task = Slug::parse("light-well-tanking").unwrap();
    assert_eq!(launching.ended_run(&task, 1), None, "nothing launched");
    let launcher = Arc::clone(&launching);
    let launched = on_a_board_thread(move || {
        launcher.launch(&request("light-well-tanking", 1), &AtomicBool::new(false))
    });
    assert!(launched.is_ok(), "{launched:?}");
    assert_eq!(launching.ended_run(&task, 1), None, "no board kept yet");
    let board = json!({
        "run": {"status": "succeeded", "budget": {"observed_tokens": 52_000}},
        "items": [{"title": "assess-failure: Assess", "state": "completed", "owner": "w1",
                   "evidence": [{"artifact": "docs/a.md", "revision": "abc"}]}],
    });
    kept.keep(
        "other-uuid",
        SwarmBoardSnapshot::object(json!({"run": null})).unwrap(),
    );
    kept.keep("7f3a-uuid", SwarmBoardSnapshot::object(board).unwrap());
    let view = launching.ended_run(&task, 1).unwrap();
    assert_eq!(
        (view.end, view.observed_tokens, view.items.len()),
        (SwarmEnd::Succeeded, 52_000, 1)
    );
    assert_eq!(view.items[0].state, SwarmItemState::Completed);
    assert_eq!(launching.ended_run(&task, 2), None, "another run");
    assert_eq!(
        launching.ended_run(&Slug::parse("other").unwrap(), 1),
        None,
        "another task"
    );
}

/// A launch refused notes no coordinator, so no board is ever read for
/// its run.
#[test]
fn a_refused_launch_reports_no_swarm() {
    let spawn = Answering::new("spawn", answered("spawn answered nothing", None));
    let kill = Answering::new("agent_cmd", answered("", None));
    let launching = Arc::new(launcher(&spawn, &kill, open_permit(vec![])));
    let launcher = Arc::clone(&launching);
    let launched =
        on_a_board_thread(move || launcher.launch(&request("a", 1), &AtomicBool::new(false)));
    assert!(launched.is_err(), "{launched:?}");
    assert_eq!(launching.ended_run(&Slug::parse("a").unwrap(), 1), None);
}
