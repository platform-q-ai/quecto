//! The board's coordinator launcher (S4c): a started task's coordinator
//! launched through this session's own `spawn` tool, as a coordinator
//! (`coordinator: true`) in a new container (`container: true`), with the
//! plan's coordinator model and effort and the brief as its task. Spawn's
//! own checks all apply, and so do the session's policy over spawn and its
//! registry's guards ([`SpawnPermit`]): the policy is asked before a start
//! writes anything and again as it launches, and the guards judge the very
//! call made, so `start` never launches what the session could not spawn.
//!
//! What spawn launched is read from its structured answer
//! ([`SpawnedChild`]), never its words. A coordinator that came up outside
//! a container, or after its call was cancelled, is stopped before the
//! launch is refused. A cancel during the launch drops spawn's work, which
//! rolls back a launch not yet registered or still awaiting its prompt
//! (#2173).
//!
//! It also tells whether this session still runs the coordinator of a
//! given run, by its label among the session's subagents. The boards the
//! final-member teardown keeps of ended runs are read back by that same
//! label ([`KeptRunReports`]).
//!
//! Blocking: a launch waits on the session's runtime for the spawn to
//! answer, at most [`LAUNCH_TIMEOUT`], on the board call's blocking thread.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::spawn_launch_ports::SpawnedChild;
use super::subagent_registry::SubagentRegistry;
use super::subagent_status::SubagentStatus;
use crate::application::project_board::ports::{
    CoordinatorLaunch, CoordinatorRequest, LaunchRefused, LaunchedCoordinator, RunLiveness,
    StartedRuns,
};
use crate::application::tools::ports::Tool;
use crate::domain::agents::services::subagent::{MAX_AGENT_ID_CHARS, validate_agent_id_format};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::sessions::entities::session::SubagentLiveness;
use crate::domain::shared::error::DomainError;
use crate::domain::tool_policy::value_objects::tool::ToolResult;
pub use kept_boards::{KeptRunReports, KeptSwarmBoards, MAX_KEPT};
#[cfg(test)]
pub(crate) use run_report::swarm_run_view;
pub use spawn_permit::SpawnPermit;

/// The longest a launch waits for spawn to answer: a container's image may
/// be built first. A launch past it is refused, and the start undone.
pub const LAUNCH_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// The longest a stop of a coordinator that came up wrong waits.
pub const KILL_TIMEOUT: Duration = Duration::from_secs(30);

/// How often a launch looks at its call's cancel while spawn works.
const CANCEL_POLL: Duration = Duration::from_millis(250);

/// Launches coordinators through `spawn` while `permit` allows it,
/// stopping one that came up wrong with `agent_cmd`; finds the ones this
/// session still runs among its `subagents`.
pub struct SpawnCoordinatorLauncher {
    spawn: Arc<dyn Tool>,
    agent_cmd: Arc<dyn Tool>,
    permit: SpawnPermit,
    subagents: SubagentRegistry,
    timeout: Duration,
    kill_timeout: Duration,
}

impl SpawnCoordinatorLauncher {
    pub fn new(
        spawn: Arc<dyn Tool>,
        agent_cmd: Arc<dyn Tool>,
        permit: SpawnPermit,
        subagents: SubagentRegistry,
    ) -> Self {
        Self {
            spawn,
            agent_cmd,
            permit,
            subagents,
            timeout: LAUNCH_TIMEOUT,
            kill_timeout: KILL_TIMEOUT,
        }
    }

    /// This launcher, waiting at most `timeout` for spawn and
    /// `kill_timeout` for a stop.
    #[must_use]
    pub fn with_timeouts(self, timeout: Duration, kill_timeout: Duration) -> Self {
        Self {
            timeout,
            kill_timeout,
            ..self
        }
    }

    /// The refusal `why` once the child `uuid` spawn launched is stopped:
    /// one that names it as still running when the stop failed or timed
    /// out, so the start leaves its run open and its claim held.
    fn stopped(&self, runtime: &tokio::runtime::Handle, why: &str, uuid: &str) -> LaunchRefused {
        let kill = json!({"agent_id": uuid, "command": "kill"}).to_string();
        // The kill runs to its end as the call's own task, even past the
        // wait: one dropped half-way would leave the child claimed as
        // stopping, so every later kill of it, and end_run, would refuse.
        let agent_cmd = Arc::clone(&self.agent_cmd);
        let killing = super::super::registry::call_work::spawn_in_call(async move {
            agent_cmd.execute(&kill).await
        });
        let stop = tokio::time::timeout(self.kill_timeout, killing);
        let failed = match runtime.block_on(stop) {
            Ok(Ok(Ok(result))) => match result.is_error {
                false => return LaunchRefused::because(format!("{why}, so it was stopped")),
                true => format!("stopping it failed: {}", result.content),
            },
            Ok(Ok(Err(error))) => format!("stopping it failed: {error}"),
            Ok(Err(error)) => format!("stopping it failed: {error}"),
            Err(_) => "stopping it timed out (the stop goes on)".into(),
        };
        LaunchRefused::leaving(format!("{why}, so {failed}"), uuid)
    }

    /// What spawn's answer `result` comes to: the coordinator running in
    /// its container, or a refusal once whatever it launched otherwise
    /// (outside a container, or after the call was cancelled) is stopped.
    fn running(
        &self,
        runtime: &tokio::runtime::Handle,
        result: ToolResult,
        cancelled: &AtomicBool,
    ) -> Result<LaunchedCoordinator, LaunchRefused> {
        let stop = |why: &str, uuid: &str| Err(self.stopped(runtime, why, uuid));
        match (SpawnedChild::of(&result), cancelled.load(Ordering::SeqCst)) {
            (Some(child), true) => stop(
                "the call was cancelled as the coordinator came up",
                &child.uuid,
            ),
            (
                Some(SpawnedChild {
                    uuid,
                    environment_ref: Some(environment_ref),
                }),
                false,
            ) => Ok(LaunchedCoordinator {
                agent: uuid,
                environment_ref,
            }),
            (
                Some(SpawnedChild {
                    uuid,
                    environment_ref: None,
                }),
                false,
            ) => stop("the coordinator came up outside a container", &uuid),
            // No structured answer: stop whatever its words name.
            (None, _) => match uuid_in(&result.content) {
                Some(uuid) => stop("spawn named no container for the coordinator", uuid),
                None => Err(LaunchRefused::because(format!(
                    "spawn answered no running coordinator: {}",
                    result.content
                ))),
            },
        }
    }
}

/// The session's launcher: through its own `spawn` and `agent_cmd`, its
/// permit observing `registry`'s policy, its coordinators found among its
/// subagents.
pub fn session_launcher(
    registry: &mut super::super::registry::ToolRegistryImpl,
    spawn: &Arc<dyn Tool>,
    agent_cmd: &Arc<dyn Tool>,
    subagents: &SubagentRegistry,
) -> SpawnCoordinatorLauncher {
    let permit = SpawnPermit::default();
    registry.observe_policy(Arc::new(permit.clone()));
    let (spawn, agent_cmd) = (Arc::clone(spawn), Arc::clone(agent_cmd));
    SpawnCoordinatorLauncher::new(spawn, agent_cmd, permit, subagents.clone())
}

/// Why a launch was refused before spawn answered.
const NOT_ALLOWED: &str = "this session's policy does not allow spawn";
const NO_RUNTIME: &str = "no runtime to launch the coordinator on";
const CANCELLED: &str =
    "the call was cancelled while the coordinator launched: the launch was dropped";

/// How a wait on spawn ended.
enum Waited {
    Answered(Result<ToolResult, DomainError>),
    Cancelled,
    TimedOut,
}

impl CoordinatorLaunch for SpawnCoordinatorLauncher {
    fn may_launch(&self) -> bool {
        self.permit.allowed()
    }

    fn launch(
        &self,
        request: &CoordinatorRequest,
        cancelled: &AtomicBool,
    ) -> Result<LaunchedCoordinator, LaunchRefused> {
        match self.permit.allowed() {
            true => {}
            false => return Err(LaunchRefused::because(NOT_ALLOWED)),
        }
        let arguments = spawn_arguments(request).to_string();
        // The registry's guards judge this call as they judge the model's.
        self.permit
            .check(&arguments)
            .map_err(LaunchRefused::because)?;
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| LaunchRefused::because(NO_RUNTIME))?;
        let waited = runtime.block_on(async {
            let mut work = std::pin::pin!(self.spawn.execute(&arguments));
            let deadline = tokio::time::Instant::now() + self.timeout;
            loop {
                if let Ok(answered) = tokio::time::timeout(CANCEL_POLL, &mut work).await {
                    break Waited::Answered(answered);
                }
                // Dropping `work` (returning) abandons spawn's launch.
                match (
                    cancelled.load(Ordering::SeqCst),
                    past(deadline, tokio::time::Instant::now()),
                ) {
                    (true, _) => break Waited::Cancelled,
                    (false, true) => break Waited::TimedOut,
                    (false, false) => {}
                }
            }
        });
        match waited {
            Waited::Answered(Ok(result)) => match result.is_error {
                false => self.running(&runtime, result, cancelled),
                true => Err(LaunchRefused::because(result.content)),
            },
            Waited::Answered(Err(error)) => Err(LaunchRefused::because(error.to_string())),
            Waited::Cancelled => Err(LaunchRefused::because(CANCELLED)),
            Waited::TimedOut => Err(LaunchRefused::because(format!(
                "spawn did not answer within {:?}",
                self.timeout
            ))),
        }
    }
}

impl RunLiveness for SpawnCoordinatorLauncher {
    /// Any subagent of this session with the run's label that runs: its
    /// agent id (its registry key, the uuid `agent_cmd kill` takes).
    fn running_coordinator(&self, task: &Slug, run: u32) -> Option<String> {
        let label = label(task.as_str(), run);
        let entries = self.subagents.lock().unwrap_or_else(|e| e.into_inner());
        entries
            .iter()
            .find(|(key, entry)| entry.effective_display_name(key) == label && runs(entry))
            .map(|(key, _)| key.clone())
    }
}

impl StartedRuns for SpawnCoordinatorLauncher {
    fn started(&self) -> Vec<(Slug, u32)> {
        vec![]
    }
}

/// Whether a subagent `entry` may run: live or detached (a detached child
/// still runs, out of reach), and in a status of one that runs (an
/// allowlist).
fn runs(entry: &super::subagent_registry::SubagentEntry) -> bool {
    let live = match entry.persisted_liveness {
        SubagentLiveness::Live | SubagentLiveness::Detached => true,
        SubagentLiveness::Dead => false,
    };
    live && match entry.status {
        SubagentStatus::Starting
        | SubagentStatus::Idle
        | SubagentStatus::Running
        | SubagentStatus::Error => true,
        SubagentStatus::Exited => false,
    }
}

/// Whether `deadline` has come by `now`: at it, or past it.
fn past(deadline: tokio::time::Instant, now: tokio::time::Instant) -> bool {
    now >= deadline
}

/// The uuid spawn's words `(uuid=…)` name, when they name one.
fn uuid_in(content: &str) -> Option<&str> {
    let (_, rest) = content.split_once("(uuid=")?;
    let (uuid, _) = rest.split_once(')')?;
    Some(uuid).filter(|uuid| uuid.chars().count() > 0)
}

/// The spawn call that launches `request`'s coordinator.
fn spawn_arguments(request: &CoordinatorRequest) -> Value {
    let mut arguments = Map::new();
    let label = label(request.task.as_str(), request.run);
    arguments.insert("agent_id".into(), json!(label));
    arguments.insert("task".into(), json!(request.brief));
    arguments.insert("container".into(), json!(true));
    arguments.insert("coordinator".into(), json!(true));
    if let Some(model) = &request.model {
        arguments.insert("model".into(), json!(model));
    }
    if let Some(effort) = request.effort {
        arguments.insert("effort".into(), json!(effort.as_str()));
    }
    Value::Object(arguments)
}

/// Whether `text` is shaped as [`label`] names a board run's coordinator:
/// `board-<task>-run-<run>`, its task a slug (a long one cut and hashed as
/// `label` cuts it is still one) and its run a number from 1, within
/// spawn's [`MAX_AGENT_ID_CHARS`]. An allowlist of the one shape the
/// board's reports read: an ad-hoc swarm's coordinator is never kept.
fn is_board_label(text: &str) -> bool {
    let shaped = text
        .strip_prefix("board-")
        .and_then(|rest| rest.rsplit_once("-run-"))
        .and_then(|(task, run)| Some((Slug::parse(task).ok()?, run.parse::<u32>().ok()?, run)));
    match shaped {
        Some((_, run, written)) if run >= 1 && run.to_string() == written => {
            text.chars().count() <= MAX_AGENT_ID_CHARS
        }
        Some(_) | None => false,
    }
}

/// The coordinator's spawn label, `board-<task>-run-<run>`, within spawn's
/// [`MAX_AGENT_ID_CHARS`]: a task id too long for it keeps its first part
/// and gains a hash of the whole, so two long ids never share a label.
fn label(task: &str, run: u32) -> String {
    let whole = format!("board-{task}-run-{run}");
    if whole.chars().count() <= MAX_AGENT_ID_CHARS {
        return whole;
    }
    let digest = Sha256::digest(task.as_bytes());
    let hash: String = digest[..4]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let tail = format!("-{hash}-run-{run}");
    let room = MAX_AGENT_ID_CHARS - "board-".len() - tail.len();
    // A slug is ASCII, so its characters are its bytes.
    let label = format!("board-{}{tail}", &task[..room.min(task.len())]);
    assert!(
        validate_agent_id_format(&label).is_ok(),
        "a board label is a valid agent_id: {label}"
    );
    label
}

mod kept_boards;
mod run_report;
mod spawn_permit;

#[cfg(test)]
#[path = "launcher_tests.rs"]
mod tests;
