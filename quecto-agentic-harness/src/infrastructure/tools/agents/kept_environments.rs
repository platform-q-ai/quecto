//! The environments a selected termination kept (#2681), read from the
//! session's environment registry: the memberships of the agents a kill
//! ends, before it, and the records retained after it, with the reason
//! each record holds ([`EnvironmentRecord::retained_reason`], the one
//! owner of that text).

use crate::application::subagents::ports::kept_environments::{KeptEnvironment, KeptEnvironments};
use crate::domain::environments::entities::environment_registry::{
    EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus,
};
use crate::domain::identity::ids::AgentUuid;

/// [`KeptEnvironments`] over one session's environment registry.
pub struct RegistryKeptEnvironments {
    registry: EnvironmentRegistry,
}

impl RegistryKeptEnvironments {
    pub fn new(registry: EnvironmentRegistry) -> Self {
        Self { registry }
    }
}

/// Whether a teardown of `record`'s members may yet keep it: one still
/// live (an allowlist). A record already kept, ending or ended was not
/// kept by the teardown to come.
fn live(record: &EnvironmentRecord) -> bool {
    match record.status {
        EnvironmentStatus::Running => true,
        EnvironmentStatus::Killing
        | EnvironmentStatus::Stopped
        | EnvironmentStatus::CleanupFailed
        | EnvironmentStatus::Retained => false,
    }
}

impl KeptEnvironments for RegistryKeptEnvironments {
    fn memberships(&self, agents: &[AgentUuid]) -> Vec<String> {
        self.registry
            .entries()
            .into_iter()
            .filter(|record| {
                live(record)
                    && record
                        .members
                        .iter()
                        .any(|member| agents.iter().any(|agent| agent.as_str() == member))
            })
            .map(|record| record.environment_ref)
            .collect()
    }

    fn kept(&self, environments: &[String]) -> Vec<KeptEnvironment> {
        environments
            .iter()
            .filter_map(|environment_ref| {
                let record = self.registry.get(environment_ref)?;
                let reason = record.retained_reason()?;
                Some(KeptEnvironment {
                    environment_ref: environment_ref.clone(),
                    reason: reason.to_owned(),
                })
            })
            .collect()
    }
}
