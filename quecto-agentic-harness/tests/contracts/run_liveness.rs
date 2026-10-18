//! The run liveness port's contract (S4c review round 3), run against its
//! spawn-backed adapter: a run's swarm still runs while a subagent of this
//! session with the run's label is live or detached and not exited, and
//! the port names it; any other run, task or subagent is not seen running.
use std::future::Future;
use std::pin::Pin;

use quecto::application::project_board::ports::RunLiveness;
use quecto::application::tools::ports::Tool;
use quecto::domain::error::DomainError;
use quecto::domain::ids::AgentUuid;
use quecto::domain::project_board::value_objects::slug::Slug;
use quecto::domain::sessions::entities::session::SubagentLiveness;
use quecto::domain::tool::{ToolDefinition, ToolResult};
use quecto::infrastructure::tools::coordinator_launch::{SpawnCoordinatorLauncher, SpawnPermit};
use quecto::infrastructure::tools::subagent_registry::{
    SubagentEntry, SubagentRegistry, SubagentStatus, new_registry,
};

/// A tool never called here.
struct Unused;

impl Tool for Unused {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "unused".into(),
            description: "never called".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }

    fn execute(
        &self,
        _: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        Box::pin(async { Err(DomainError::Tool("never called".into())) })
    }
}

/// The session's subagents, holding one labelled `label` as `liveness`
/// and `status`; its key.
fn holding(
    subagents: &SubagentRegistry,
    label: &str,
    liveness: SubagentLiveness,
    status: SubagentStatus,
) -> String {
    let uuid = AgentUuid::mint();
    let mut entry =
        SubagentEntry::with_identity(uuid.clone(), label.into(), "/tmp/c.sock".into(), 0);
    entry.persisted_liveness = liveness;
    entry.status = status;
    let key = uuid.as_str().to_owned();
    subagents.lock().unwrap().insert(key.clone(), entry);
    key
}

fn liveness(subagents: &SubagentRegistry) -> SpawnCoordinatorLauncher {
    let unused = || std::sync::Arc::new(Unused) as std::sync::Arc<dyn Tool>;
    SpawnCoordinatorLauncher::new(
        unused(),
        unused(),
        SpawnPermit::default(),
        subagents.clone(),
    )
}

#[test]
fn a_runs_swarm_runs_while_its_coordinator_is_live() {
    let subagents = new_registry();
    let key = holding(
        &subagents,
        "board-demo-hello-run-1",
        SubagentLiveness::Live,
        SubagentStatus::Running,
    );
    let seen = liveness(&subagents);
    let task = Slug::parse("demo-hello").unwrap();
    assert_eq!(seen.running_coordinator(&task, 1), Some(key.clone()));
    assert_eq!(seen.running_coordinator(&task, 2), None, "another run's");
    assert_eq!(
        seen.running_coordinator(&Slug::parse("other").unwrap(), 1),
        None,
        "another task's"
    );
    subagents
        .lock()
        .unwrap()
        .get_mut(&key)
        .unwrap()
        .persisted_liveness = SubagentLiveness::Detached;
    assert_eq!(
        seen.running_coordinator(&task, 1),
        Some(key.clone()),
        "a detached one still runs"
    );
    subagents.lock().unwrap().get_mut(&key).unwrap().status = SubagentStatus::Exited;
    assert_eq!(
        seen.running_coordinator(&task, 1),
        None,
        "an exited one runs nothing"
    );
}

#[test]
fn a_dead_coordinator_runs_nothing_and_neither_does_another_label() {
    let subagents = new_registry();
    let seen = liveness(&subagents);
    let task = Slug::parse("demo-hello").unwrap();
    holding(
        &subagents,
        "board-demo-hello-run-1",
        SubagentLiveness::Dead,
        SubagentStatus::Idle,
    );
    holding(
        &subagents,
        "worker-demo-hello-run-1",
        SubagentLiveness::Live,
        SubagentStatus::Running,
    );
    assert_eq!(seen.running_coordinator(&task, 1), None);
}
