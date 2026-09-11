//! Application use case for final-member environment cleanup.
//!
//! Membership removal may mint the exclusive final cleanup claim atomically in
//! the domain registry. This use case owns the surrounding application
//! transaction; infrastructure supplies only the retained script executions
//! and the observation of a hosted swarm run (#1924).

use std::sync::Arc;

use crate::domain::environment_registry::{EnvironmentRecord, EnvironmentRegistry};
use crate::domain::subagent_launch::LaunchFuture;
use crate::domain::swarm::RunStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberFinalizeMode {
    Exit,
    ParentKill,
    LaunchRollback,
    LaunchRollbackOwned,
}

/// The swarm run an environment hosts, as observed from outside it (#1924).
#[derive(Debug, Clone, PartialEq)]
pub struct HostedSwarmRun {
    pub status: RunStatus,
    pub coordinator: String,
    /// Run deadline; the bootstrap placeholder carries 0 (no swarm created).
    pub deadline: f64,
}

impl HostedSwarmRun {
    /// A created run that is still resumable: running, or paused (holding an
    /// outcome or not) for the supervisor outside the swarm to decide (#1729).
    /// Terminal runs and the bootstrap placeholder are not.
    pub fn resumable(&self) -> bool {
        crate::domain::swarm::participates(self.deadline)
            && matches!(self.status, RunStatus::Running | RunStatus::Paused)
    }
}

/// What the supervising session could learn about the swarm run an
/// environment hosts (#1924).
#[derive(Debug, Clone, PartialEq)]
pub enum SwarmRunObservation {
    /// The environment advertises no coordination store, or none exists yet:
    /// an ordinary container.
    NoStore,
    /// The store's run.
    Run(HostedSwarmRun),
    /// A store exists but could not be read (contended, corrupt, no
    /// interpreter). A store exists only where members coordinate, so this
    /// is not proof that no run is live.
    Unreadable(String),
}

/// Whether the final-member teardown of an environment must be withheld
/// (#1924): a member whose exit empties the environment while it hosts a
/// resumable swarm run is its coordinator (workers are the coordinator's
/// in-container descendants, never members of the supervising session's
/// environment record). Destroying the box would lose the board, the checkout
/// and unpushed work; #1729 requires every swarm end to be a resumable pause
/// only the supervisor closes. A store that exists but cannot be read is
/// retained too: an explicit `kill_container` can always close it later,
/// while a destroyed box cannot be recovered. Rollbacks and parent-initiated
/// kills keep their existing behaviour: the supervisor asked for those.
pub fn retains_environment(mode: MemberFinalizeMode, observed: &SwarmRunObservation) -> bool {
    mode == MemberFinalizeMode::Exit
        && match observed {
            SwarmRunObservation::NoStore => false,
            SwarmRunObservation::Run(hosted) => hosted.resumable(),
            SwarmRunObservation::Unreadable(_) => true,
        }
}

pub trait EnvironmentFinalizationPort: Send + Sync {
    /// Observe the swarm run `record` hosts, when it advertises a
    /// coordination store this session can reach. The default (no store)
    /// keeps the ordinary final-member teardown.
    fn observe_hosted_swarm_run<'a>(
        &'a self,
        record: &'a EnvironmentRecord,
    ) -> LaunchFuture<'a, SwarmRunObservation> {
        let _ = record;
        Box::pin(async { SwarmRunObservation::NoStore })
    }

    /// Record the coordinator's harness as lost on the hosted run, ending it
    /// as a pause holding `failed` (the #1729 lost-harness rule).
    fn record_lost_coordinator<'a>(
        &'a self,
        record: &'a EnvironmentRecord,
        hosted: &'a HostedSwarmRun,
    ) -> LaunchFuture<'a, Result<(), String>> {
        let _ = (record, hosted);
        Box::pin(async { Ok(()) })
    }

    fn run_retained_inspect<'a>(
        &'a self,
        environment_id: &'a str,
        argv: &'a [String],
    ) -> LaunchFuture<'a, Result<serde_json::Value, String>>;

    fn run_retained_kill<'a>(
        &'a self,
        environment_id: &'a str,
        argv: &'a [String],
    ) -> LaunchFuture<'a, Result<(), String>>;

    fn run_retained_cleanup<'a>(
        &'a self,
        environment_id: &'a str,
        argv: &'a [String],
    ) -> LaunchFuture<'a, ()>;
}

pub struct EnvironmentFinalizationUseCase {
    registry: EnvironmentRegistry,
    port: Arc<dyn EnvironmentFinalizationPort>,
}

impl EnvironmentFinalizationUseCase {
    pub fn new(registry: EnvironmentRegistry, port: Arc<dyn EnvironmentFinalizationPort>) -> Self {
        Self { registry, port }
    }

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

        let Ok(removal) = self.registry.remove_member(env_ref, agent_uuid) else {
            return;
        };
        let Some(claim) = removal else {
            return;
        };
        let Some(record) = self.registry.get(env_ref) else {
            self.registry.complete_kill(claim);
            return;
        };

        let observed = if mode == MemberFinalizeMode::Exit {
            self.port.observe_hosted_swarm_run(&record).await
        } else {
            SwarmRunObservation::NoStore
        };
        if retains_environment(mode, &observed) {
            self.retain_for_lost_coordinator(claim, &record, &observed)
                .await;
            return;
        }

        let launch_rollback = matches!(
            mode,
            MemberFinalizeMode::LaunchRollback | MemberFinalizeMode::LaunchRollbackOwned
        );
        let run_cleanup = launch_rollback || record.retained_kill_argv.is_empty();
        if run_cleanup {
            if !record.retained_cleanup_argv.is_empty() {
                self.port
                    .run_retained_cleanup(&record.environment_id, &record.retained_cleanup_argv)
                    .await;
            } else if let Some((env_id, argv)) = entry_cleanup_plan {
                self.port.run_retained_cleanup(&env_id, &argv).await;
            }
            self.registry.complete_kill(claim);
            if mode == MemberFinalizeMode::LaunchRollbackOwned {
                self.registry.remove(env_ref);
            }
            return;
        }

        match self
            .port
            .run_retained_kill(&record.environment_id, &record.retained_kill_argv)
            .await
        {
            Ok(()) => self.registry.complete_kill(claim),
            Err(e) => self.registry.fail_kill(claim, &e),
        }
    }

    /// The retained kill is withheld and the hosted run is paused holding
    /// `failed` (#1924). A store that refuses the loss record still leaves the
    /// environment retained: losing the box is never the safer outcome.
    async fn retain_for_lost_coordinator(
        &self,
        claim: crate::domain::environment_registry::KillClaim,
        record: &EnvironmentRecord,
        observed: &SwarmRunObservation,
    ) {
        const KEPT: &str = "environment kept for inspection or recovery (kill_container closes it)";
        let reason = match observed {
            SwarmRunObservation::Run(hosted) => {
                match self.port.record_lost_coordinator(record, hosted).await {
                    Ok(()) => format!(
                        "swarm coordinator '{}' lost its connection while the run was {:?}; run paused holding failed; {KEPT}",
                        hosted.coordinator, hosted.status
                    ),
                    Err(error) => format!(
                        "swarm coordinator '{}' lost its connection while the run was {:?}; the run could not be paused ({error}); {KEPT}",
                        hosted.coordinator, hosted.status
                    ),
                }
            }
            SwarmRunObservation::Unreadable(error) => format!(
                "final member lost its connection while the coordination store could not be read ({error}); {KEPT}"
            ),
            SwarmRunObservation::NoStore => {
                debug_assert!(false, "retention never applies without a store");
                format!("final member lost its connection; {KEPT}")
            }
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
            .port
            .run_retained_inspect(&record.environment_id, &record.retained_inspect_argv)
            .await
        {
            Ok(metadata) => self.registry.record_inspect_success(claim, metadata),
            Err(e) => self.registry.record_inspect_failure(claim, &e),
        }
    }
}
