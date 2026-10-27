//! Whether an agent takes part in a swarm, and what a swarm member may not
//! run alongside it (#1715).

use crate::domain::shared::error::DomainError;

/// A swarm owns its coordination lifecycle; workflow engines cannot run alongside it.
pub fn validate_workflow(swarm_agent: bool, requested: bool) -> Result<(), DomainError> {
    if swarm_agent && requested {
        return Err(DomainError::Tool("workflow is unavailable for swarm agents; omit workflow, workflow_guards and workflow_spec".into()));
    }
    Ok(())
}

/// Whether an agent in a container takes part in a swarm (#1715). Every
/// container carries a placeholder run from its bootstrap with deadline 0;
/// that is an ordinary container whatever status it ends in. `create`
/// requires a future deadline, and a created run keeps it through every later
/// status, so a positive deadline means a swarm exists and every member is a
/// swarm agent.
pub fn participates(deadline: f64) -> bool {
    deadline > 0.0
}

/// How this process was launched, as swarm creation judges it
/// (swarm-container-matches-repo).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchRole {
    /// Launched with `--coordinator`: by a spawn with `coordinator: true`
    /// (a board start's included), whose new container was confirmed to
    /// clone the launching project's own repository.
    Coordinator,
    /// Any other process: a host agent, an ordinary container child, a
    /// member.
    Other,
}

impl LaunchRole {
    /// The role the `--coordinator` launch flag names.
    pub fn of_flag(coordinator: bool) -> Self {
        match coordinator {
            true => Self::Coordinator,
            false => Self::Other,
        }
    }
}

/// Why a process not launched as a coordinator creates no swarm.
pub const NOT_A_COORDINATOR: &str = "swarm create refused: only an agent launched as a swarm coordinator (spawn with coordinator: true, as board start does) can create a swarm, and this agent was not; ask your parent to launch the swarm's coordinator instead";

/// Creating a swarm turns the creator into its coordinator: only a process
/// launched as one may (an allowlist: its container's repository was
/// confirmed at launch), and it cannot be running a workflow (guards, a
/// bound spec or a selected template). An idle, merely available workflow
/// tool does not block creation; it refuses to act afterwards.
pub fn validate_swarm_creation(
    workflow_engaged: bool,
    launch: LaunchRole,
) -> Result<(), DomainError> {
    match launch {
        LaunchRole::Coordinator => {}
        LaunchRole::Other => return Err(DomainError::Tool(NOT_A_COORDINATOR.into())),
    }
    if workflow_engaged {
        return Err(DomainError::Tool(
            "a workflow-enabled agent cannot create a swarm; finish or relaunch without workflow, workflow_guards and workflow_spec, then create the run".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "participation_tests.rs"]
mod tests;
