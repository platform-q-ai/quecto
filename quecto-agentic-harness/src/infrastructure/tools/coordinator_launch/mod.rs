//! The board's coordinator launcher (S4c): a started task's coordinator
//! launched through this session's own `spawn` tool, as a coordinator
//! (`coordinator: true`) in a new container (`container: true`), with the
//! plan's coordinator model and effort and the brief as its task. Spawn's
//! own checks all apply; this adds the session's policy over spawn
//! ([`SpawnPermit`]), asked before a start writes anything and again as it
//! launches, so `start` never launches what the session could not spawn.
//!
//! Blocking: a launch waits on the session's runtime for the spawn to
//! answer, on the board call's blocking thread.
use std::sync::Arc;

use serde_json::{Map, Value, json};

use crate::application::project_board::ports::{
    CoordinatorLaunch, CoordinatorRequest, LaunchRefused, LaunchedCoordinator,
};
use crate::application::tools::ports::Tool;
pub use spawn_permit::SpawnPermit;

/// Launches coordinators through `spawn` while `permit` allows it.
pub struct SpawnCoordinatorLauncher {
    spawn: Arc<dyn Tool>,
    permit: SpawnPermit,
}

impl SpawnCoordinatorLauncher {
    pub fn new(spawn: Arc<dyn Tool>, permit: SpawnPermit) -> Self {
        Self { spawn, permit }
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
        let runtime =
            tokio::runtime::Handle::try_current().map_err(|_| LaunchRefused(NO_RUNTIME.into()))?;
        let arguments = spawn_arguments(request).to_string();
        let answered = runtime.block_on(self.spawn.execute(&arguments));
        match answered {
            Ok(result) if !result.is_error => launched(&result.content).ok_or_else(|| {
                LaunchRefused(format!(
                    "spawn answered no running coordinator: {}",
                    result.content
                ))
            }),
            Ok(result) => Err(LaunchRefused(result.content)),
            Err(error) => Err(LaunchRefused(error.to_string())),
        }
    }
}

/// The spawn call that launches `request`'s coordinator.
fn spawn_arguments(request: &CoordinatorRequest) -> Value {
    let mut arguments = Map::new();
    let label = format!("board-{}-run-{}", request.task.as_str(), request.run);
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

/// The coordinator a spawn's answer says runs: its `uuid` and its
/// container's `environment_ref`, as spawn's running line spells them; none
/// when either is missing (a coordinator runs in a container).
fn launched(content: &str) -> Option<LaunchedCoordinator> {
    let value_of = |key: &str, end: &[char]| {
        let start = content.find(key)? + key.len();
        let rest = &content[start..];
        let value = &rest[..rest.find(end).unwrap_or(rest.len())];
        (!value.is_empty()).then(|| value.to_owned())
    };
    Some(LaunchedCoordinator {
        agent: value_of("(uuid=", &[')'])?,
        environment_ref: value_of(" environment_ref=", &[' ', '.', '\n'])?,
    })
}

mod spawn_permit;

#[cfg(test)]
#[path = "launcher_tests.rs"]
mod tests;
