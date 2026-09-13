//! Final-member environment cleanup (#1369, #1924, #1939).
//!
//! Membership removal may mint the exclusive final cleanup claim atomically
//! in the domain registry. This use case owns the surrounding application
//! transaction: the one post-mortem inspect per dead member, the retention
//! decision (the domain's), the coordinator-loss record, and the retained
//! `kill` / `cleanup` runs. Infrastructure supplies only the script
//! executions and the observation of a hosted swarm run through the
//! capability's ports.
use std::sync::Arc;

use crate::domain::environment_registry::{EnvironmentRecord, EnvironmentRegistry, KillClaim};
use crate::domain::environment_retention::{
    MemberFinalizeMode, SwarmRunObservation, loss_reason, retains_environment, retention_reason,
    unrecorded_loss_reason,
};

use super::super::ports::{EnvironmentProcessCommands, HostedSwarmRunObservation};

pub struct FinalizeEnvironmentMember {
    registry: EnvironmentRegistry,
    commands: Arc<dyn EnvironmentProcessCommands>,
    hosted: Arc<dyn HostedSwarmRunObservation>,
}

impl FinalizeEnvironmentMember {
    pub fn new(
        registry: EnvironmentRegistry,
        commands: Arc<dyn EnvironmentProcessCommands>,
        hosted: Arc<dyn HostedSwarmRunObservation>,
    ) -> Self {
        Self {
            registry,
            commands,
            hosted,
        }
    }

    /// Remove `agent_uuid` from `env_ref`; when that empties a running
    /// environment, run its final teardown exactly once under the claim the
    /// removal minted. `entry_cleanup_plan` is the member's own retained
    /// cleanup (creator's record) used only when the environment retained no
    /// cleanup argv of its own.
    pub async fn finalize_member(
        &self,
        env_ref: &str,
        agent_uuid: &str,
        entry_cleanup_plan: Option<(String, Vec<String>)>,
        mode: MemberFinalizeMode,
    ) {
        if mode == MemberFinalizeMode::Exit {
            self.inspect_once(env_ref, agent_uuid).await;
        }

        let Ok(Some(claim)) = self.registry.remove_member(env_ref, agent_uuid) else {
            return;
        };
        let Some(record) = self.registry.get(env_ref) else {
            self.registry.complete_kill(claim);
            return;
        };

        let observed = if mode.inspectable_end() {
            self.hosted.observe_hosted_swarm_run(&record).await
        } else {
            SwarmRunObservation::NoStore
        };
        if retains_environment(mode, &observed) {
            self.retain(claim, &record, mode, &observed).await;
            return;
        }

        if mode.launch_rollback() || record.retained_kill_argv.is_empty() {
            self.cleanup(claim, &record, entry_cleanup_plan, mode).await;
            return;
        }

        match self
            .commands
            .run_retained_kill(&record.environment_id, &record.retained_kill_argv)
            .await
        {
            Ok(()) => self.registry.complete_kill(claim),
            Err(e) => self.registry.fail_kill(claim, &e),
        }
    }

    /// The retained `cleanup` contract: the environment's own cleanup argv,
    /// else the member's plan; then stopped, and for the launch that created
    /// the environment the record is discarded entirely.
    async fn cleanup(
        &self,
        claim: KillClaim,
        record: &EnvironmentRecord,
        entry_cleanup_plan: Option<(String, Vec<String>)>,
        mode: MemberFinalizeMode,
    ) {
        if !record.retained_cleanup_argv.is_empty() {
            self.commands
                .run_retained_cleanup(&record.environment_id, &record.retained_cleanup_argv)
                .await;
        } else if let Some((env_id, argv)) = entry_cleanup_plan {
            self.commands.run_retained_cleanup(&env_id, &argv).await;
        }
        self.registry.complete_kill(claim);
        if mode == MemberFinalizeMode::LaunchRollbackOwned {
            self.registry.remove(&record.environment_ref);
        }
    }

    /// The retained kill is withheld (#1924). On the member's own exit a
    /// created run that has not ended is recorded as a coordinator loss
    /// through the port (one store operation decides); a supervisor's kill
    /// only records what it found. A store that refuses the loss record still
    /// leaves the environment retained: losing the box is never the safer
    /// outcome.
    async fn retain(
        &self,
        claim: KillClaim,
        record: &EnvironmentRecord,
        mode: MemberFinalizeMode,
        observed: &SwarmRunObservation,
    ) {
        let reason = match observed {
            SwarmRunObservation::Run(hosted) if mode == MemberFinalizeMode::Exit => {
                match self.hosted.record_lost_coordinator(record, hosted).await {
                    Ok(loss) => loss_reason(&hosted.coordinator, &loss),
                    Err(error) => unrecorded_loss_reason(hosted, &error),
                }
            }
            observed => retention_reason(mode, observed),
        };
        self.registry.retain(claim, &reason);
    }

    async fn inspect_once(&self, env_ref: &str, agent_uuid: &str) {
        let Some(record) = self.registry.get(env_ref) else {
            return;
        };
        if record.retained_inspect_argv.is_empty() {
            return;
        }
        let Some(claim) = self.registry.begin_inspect(env_ref, agent_uuid) else {
            return;
        };
        match self
            .commands
            .run_retained_inspect(&record.environment_id, &record.retained_inspect_argv)
            .await
        {
            Ok(metadata) => self.registry.record_inspect_success(claim, metadata),
            Err(e) => self.registry.record_inspect_failure(claim, &e),
        }
    }
}
