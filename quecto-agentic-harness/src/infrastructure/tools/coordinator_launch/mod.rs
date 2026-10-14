//! The board's coordinator launcher (S4c): a started task's coordinator
//! launched through this session's own `spawn` tool, as a coordinator
//! (`coordinator: true`) in a new container (`container: true`), with the
//! plan's coordinator model and effort and the brief as its task. Spawn's
//! own checks all apply, and so do the session's policy over spawn and its
//! registry's guards ([`SpawnPermit`]): the policy is asked before a start
//! writes anything and again as it launches, and the guards judge the very
//! call made, so `start` never launches what the session could not spawn.
//!
//! What spawn launched is read from its structured answer (its delivery
//! metadata), never its words. A coordinator that came up outside a
//! container is stopped before the launch is refused.
//!
//! Blocking: a launch waits on the session's runtime for the spawn to
//! answer, at most [`LAUNCH_TIMEOUT`], on the board call's blocking thread.
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::application::project_board::ports::{
    CoordinatorLaunch, CoordinatorRequest, LaunchRefused, LaunchedCoordinator,
};
use crate::application::tools::ports::Tool;
use crate::domain::agents::services::subagent::{MAX_AGENT_ID_CHARS, validate_agent_id_format};
use crate::domain::tool::ToolResult;
pub use spawn_permit::SpawnPermit;

/// The longest a launch waits for spawn to answer: a container's image may
/// be built first. A launch past it is refused, and the start undone.
pub const LAUNCH_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// Launches coordinators through `spawn` while `permit` allows it,
/// stopping one that came up wrong with `agent_cmd`.
pub struct SpawnCoordinatorLauncher {
    spawn: Arc<dyn Tool>,
    agent_cmd: Arc<dyn Tool>,
    permit: SpawnPermit,
    timeout: Duration,
}

impl SpawnCoordinatorLauncher {
    pub fn new(spawn: Arc<dyn Tool>, agent_cmd: Arc<dyn Tool>, permit: SpawnPermit) -> Self {
        Self {
            spawn,
            agent_cmd,
            permit,
            timeout: LAUNCH_TIMEOUT,
        }
    }

    /// This launcher, waiting at most `timeout` for spawn.
    #[must_use]
    pub fn with_timeout(self, timeout: Duration) -> Self {
        Self { timeout, ..self }
    }

    /// Stops the child `uuid` spawn launched, telling how that went.
    fn stopped(&self, runtime: &tokio::runtime::Handle, uuid: &str) -> String {
        let kill = json!({"agent_id": uuid, "command": "kill"}).to_string();
        match runtime.block_on(tokio::time::timeout(
            self.timeout,
            self.agent_cmd.execute(&kill),
        )) {
            Ok(Ok(result)) if !result.is_error => "it was stopped".into(),
            Ok(Ok(result)) => format!("stopping it failed: {}", result.content),
            Ok(Err(error)) => format!("stopping it failed: {error}"),
            Err(_) => "stopping it timed out".into(),
        }
    }
}

/// Why a launch was refused before spawn was asked.
const NOT_ALLOWED: &str = "this session's policy does not allow spawn";
const NO_RUNTIME: &str = "no runtime to launch the coordinator on";

impl CoordinatorLaunch for SpawnCoordinatorLauncher {
    fn may_launch(&self) -> bool {
        self.permit.allowed()
    }

    fn launch(&self, request: &CoordinatorRequest) -> Result<LaunchedCoordinator, LaunchRefused> {
        if !self.permit.allowed() {
            return Err(LaunchRefused(NOT_ALLOWED.into()));
        }
        let arguments = spawn_arguments(request).to_string();
        // The registry's guards judge this call as they judge the model's.
        self.permit.check(&arguments).map_err(LaunchRefused)?;
        let runtime =
            tokio::runtime::Handle::try_current().map_err(|_| LaunchRefused(NO_RUNTIME.into()))?;
        let answered = runtime.block_on(tokio::time::timeout(
            self.timeout,
            self.spawn.execute(&arguments),
        ));
        let result = match answered {
            Ok(Ok(result)) if !result.is_error => result,
            Ok(Ok(result)) => return Err(LaunchRefused(result.content)),
            Ok(Err(error)) => return Err(LaunchRefused(error.to_string())),
            Err(_) => {
                return Err(LaunchRefused(format!(
                    "spawn did not answer within {:?}",
                    self.timeout
                )));
            }
        };
        match spawned(&result) {
            Some(Spawned {
                uuid,
                environment_ref: Some(environment_ref),
            }) => Ok(LaunchedCoordinator {
                agent: uuid,
                environment_ref,
            }),
            Some(Spawned {
                uuid,
                environment_ref: None,
            }) => Err(LaunchRefused(format!(
                "the coordinator came up outside a container, so {}",
                self.stopped(&runtime, &uuid)
            ))),
            None => Err(LaunchRefused(format!(
                "spawn answered no running coordinator: {}",
                result.content
            ))),
        }
    }
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

/// What spawn's structured answer says it launched.
#[derive(Debug, PartialEq, Eq)]
struct Spawned {
    uuid: String,
    environment_ref: Option<String>,
}

/// The child `result` names in its delivery metadata, spawn's
/// machine-readable answer; none when it names none.
fn spawned(result: &ToolResult) -> Option<Spawned> {
    let metadata: Value = serde_json::from_str(result.delivery_metadata.as_deref()?).ok()?;
    let spawned = metadata.get("spawned")?;
    let text = |key: &str| {
        spawned
            .get(key)
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    Some(Spawned {
        uuid: text("uuid")?,
        environment_ref: text("environment_ref"),
    })
}

mod spawn_permit;

#[cfg(test)]
#[path = "launcher_tests.rs"]
mod tests;
