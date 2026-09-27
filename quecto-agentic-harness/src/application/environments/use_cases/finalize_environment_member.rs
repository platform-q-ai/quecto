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
    MemberFinalizeMode, SwarmRunObservation, ends_plain_environment, loss_reason,
    retains_environment, retention_reason, unrecorded_loss_reason,
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
            // `None`: the owner closed the run between the observation and
            // the loss record (#2070) — the swarm is over, the kill runs.
            if let Some(reason) = self.retention(&record, mode, &observed).await {
                self.registry.retain(claim, &reason);
                return;
            }
        }

        // #2206: on the owner's word a plain container child's box has
        // nothing to resume — it goes for good, record included.
        if ends_plain_environment(mode, &observed) {
            let plan = Self::cleanup_plan(&record, entry_cleanup_plan.clone());
            if let Some((environment_id, argv)) = plan {
                self.end_for_good(claim, &record, &environment_id, &argv)
                    .await;
                return;
            }
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

    /// The cleanup that ends `record`: the environment's own retained
    /// cleanup argv, else the member's plan; `None` when neither names one.
    fn cleanup_plan(
        record: &EnvironmentRecord,
        entry_cleanup_plan: Option<(String, Vec<String>)>,
    ) -> Option<(String, Vec<String>)> {
        match record.retained_cleanup_argv.first() {
            Some(_program) => Some((
                record.environment_id.clone(),
                record.retained_cleanup_argv.clone(),
            )),
            None => {
                entry_cleanup_plan.filter(|(_, argv)| matches!(argv.as_slice(), [_program, ..]))
            }
        }
    }

    /// End a plain container child's environment for good (#2206): the
    /// retained `cleanup` removes the container and its state directory,
    /// and only once it reported success is the record forgotten, as a
    /// launch rollback does. A failed cleanup leaves the record
    /// `cleanup-failed` with the script's account, so `container kill`
    /// retries it and `ls` still shows what is left.
    async fn end_for_good(
        &self,
        claim: KillClaim,
        record: &EnvironmentRecord,
        environment_id: &str,
        argv: &[String],
    ) {
        debug_assert!(!argv.is_empty(), "only a named cleanup ends a box for good");
        match self
            .commands
            .run_retained_cleanup(environment_id, argv)
            .await
        {
            Ok(()) => {
                self.registry.complete_kill(claim);
                self.registry.remove(&record.environment_ref);
            }
            Err(error) => self.registry.fail_kill(claim, &error),
        }
    }

    /// The retained `cleanup` contract: the environment's own cleanup argv,
    /// else the member's plan; then stopped, and for the launch that created
    /// the environment the record is discarded entirely. Best effort: a
    /// rollback has nothing usable to keep, so its outcome only is logged.
    async fn cleanup(
        &self,
        claim: KillClaim,
        record: &EnvironmentRecord,
        entry_cleanup_plan: Option<(String, Vec<String>)>,
        mode: MemberFinalizeMode,
    ) {
        if let Some((environment_id, argv)) = Self::cleanup_plan(record, entry_cleanup_plan) {
            // The adapter reports a failure itself; best effort by contract.
            let _ = self
                .commands
                .run_retained_cleanup(&environment_id, &argv)
                .await;
        }
        self.registry.complete_kill(claim);
        if mode == MemberFinalizeMode::LaunchRollbackOwned {
            self.registry.remove(&record.environment_ref);
        }
    }

    /// The reason the retained kill is withheld (#1924), or `None` when it
    /// must run after all. On the member's own exit a created run that has
    /// not ended is recorded as a coordinator loss through the port (one
    /// store operation decides); a supervisor's kill only records what it
    /// found. A store that refuses the loss record still leaves the
    /// environment retained: losing the box is never the safer outcome. The
    /// one exception is a run its owner closed in between (#2070).
    async fn retention(
        &self,
        record: &EnvironmentRecord,
        mode: MemberFinalizeMode,
        observed: &SwarmRunObservation,
    ) -> Option<String> {
        Some(match observed {
            SwarmRunObservation::Run(hosted) if mode == MemberFinalizeMode::Exit => {
                match self.hosted.record_lost_coordinator(record, hosted).await {
                    Ok(loss) if !loss.lost && !loss.run.keeps_environment() => return None,
                    Ok(loss) => loss_reason(&hosted.coordinator, &loss),
                    Err(error) => unrecorded_loss_reason(hosted, &error),
                }
            }
            observed => retention_reason(mode, observed),
        })
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
