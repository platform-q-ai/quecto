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
    /// The outcome a paused run holds after an orderly end (#1729); `None`
    /// for a live run or a plain supervisor pause.
    pub outcome: Option<RunStatus>,
    pub coordinator: String,
    /// Run deadline; the bootstrap placeholder carries 0 (no swarm created).
    pub deadline: f64,
}

impl HostedSwarmRun {
    /// A run the coordinator actually created (#1715): the bootstrap
    /// placeholder every container carries has deadline 0 and is no swarm.
    pub fn created(&self) -> bool {
        crate::domain::swarm::participates(self.deadline)
    }

    /// The run ended in an orderly way (paused holding a proposable outcome,
    /// closed into that outcome, or cancelled): the coordinator's socket
    /// closing afterwards is not a loss.
    pub fn ended(&self) -> bool {
        self.status.terminal()
            || (self.status == RunStatus::Paused && self.outcome.is_some_and(RunStatus::proposable))
    }

    /// Operator-facing description of the run's state.
    pub fn describe(&self) -> String {
        match (self.status, self.outcome) {
            (RunStatus::Paused, Some(outcome)) => {
                format!("paused holding {}", status_name(outcome))
            }
            (status, _) => status_name(status).to_string(),
        }
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

/// The result of recording a coordinator loss on the store in one operation
/// (#1924): the run as it stands afterwards, and whether the loss was
/// actually recorded (a run that had already ended is left alone).
#[derive(Debug, Clone, PartialEq)]
pub struct CoordinatorLoss {
    pub run: HostedSwarmRun,
    pub lost: bool,
}

/// Whether the final-member teardown of an environment must be withheld
/// (#1924): a member whose exit empties the environment while it hosts a
/// created swarm run is its coordinator (workers are the coordinator's
/// in-container descendants, never members of the supervising session's
/// environment record). The box is kept after EVERY swarm end — orderly,
/// closed, cancelled, or by loss of the coordinator — because the full end
/// state of a swarm (board, checkout, unpushed work, logs) is worth
/// inspecting; only an explicit `kill_container` tears it down. That holds
/// for the member's own exit and for a supervisor's `kill` of the member or
/// shutdown of the master alike; only a launch rollback (nothing to inspect)
/// keeps its cleanup. A store that exists but cannot be read is retained too:
/// an explicit kill can always close it later, while a destroyed box cannot
/// be recovered.
pub fn retains_environment(mode: MemberFinalizeMode, observed: &SwarmRunObservation) -> bool {
    matches!(
        mode,
        MemberFinalizeMode::Exit | MemberFinalizeMode::ParentKill
    ) && match observed {
        SwarmRunObservation::NoStore => false,
        SwarmRunObservation::Run(hosted) => hosted.created(),
        SwarmRunObservation::Unreadable(_) => true,
    }
}

const KEPT: &str = "environment retained for inspection, kill_container to remove";

/// The `metadata.retained` reason for a withheld teardown (#1924).
pub fn retention_reason(mode: MemberFinalizeMode, observed: &SwarmRunObservation) -> String {
    match observed {
        SwarmRunObservation::Unreadable(error) => format!(
            "final member gone while the coordination store could not be read ({error}); {KEPT}"
        ),
        SwarmRunObservation::NoStore => format!("final member gone; {KEPT}"),
        SwarmRunObservation::Run(run) if mode == MemberFinalizeMode::ParentKill => format!(
            "coordinator killed by supervisor; run {}; {KEPT}",
            run.describe()
        ),
        SwarmRunObservation::Run(run) => match (run.status, run.outcome) {
            (RunStatus::Cancelled, _) => format!("run ended: cancelled; {KEPT}"),
            (status, _) if status.terminal() => {
                format!("run closed: {}; {KEPT}", status_name(status))
            }
            (RunStatus::Paused, Some(outcome)) => {
                format!("run ended: {}; {KEPT}", status_name(outcome))
            }
            _ => format!("run {}; {KEPT}", run.describe()),
        },
    }
}

/// The reason once a coordinator loss was recorded through the port.
fn loss_reason(coordinator: &str, loss: &CoordinatorLoss) -> String {
    if loss.lost {
        format!(
            "swarm coordinator '{coordinator}' lost its connection; run {}; {KEPT}",
            loss.run.describe()
        )
    } else {
        retention_reason(
            MemberFinalizeMode::Exit,
            &SwarmRunObservation::Run(loss.run.clone()),
        )
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

    /// Record the coordinator's harness as lost on the hosted run in ONE store
    /// operation: a run that has already ended (including one the store's own
    /// expiry check ends first) is left alone; any other run is paused
    /// holding `failed` (the #1729 lost-harness rule). Returns the run as it
    /// stands afterwards.
    fn record_lost_coordinator<'a>(
        &'a self,
        record: &'a EnvironmentRecord,
        hosted: &'a HostedSwarmRun,
    ) -> LaunchFuture<'a, Result<CoordinatorLoss, String>> {
        let _ = record;
        Box::pin(async move {
            Ok(CoordinatorLoss {
                run: hosted.clone(),
                lost: false,
            })
        })
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

        let observed = if matches!(
            mode,
            MemberFinalizeMode::Exit | MemberFinalizeMode::ParentKill
        ) {
            self.port.observe_hosted_swarm_run(&record).await
        } else {
            SwarmRunObservation::NoStore
        };
        if retains_environment(mode, &observed) {
            self.retain(claim, &record, mode, &observed).await;
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

    /// The retained kill is withheld (#1924). On the member's own exit a
    /// created run that has not ended is recorded as a coordinator loss
    /// through the port (one store operation decides); a supervisor's kill
    /// only records what it found. A store that refuses the loss record still
    /// leaves the environment retained: losing the box is never the safer
    /// outcome.
    async fn retain(
        &self,
        claim: crate::domain::environment_registry::KillClaim,
        record: &EnvironmentRecord,
        mode: MemberFinalizeMode,
        observed: &SwarmRunObservation,
    ) {
        let reason = match observed {
            SwarmRunObservation::Run(hosted) if mode == MemberFinalizeMode::Exit => {
                match self.port.record_lost_coordinator(record, hosted).await {
                    Ok(loss) => loss_reason(&hosted.coordinator, &loss),
                    Err(error) => format!(
                        "swarm coordinator '{}' lost its connection while the run was {}; the loss could not be recorded ({error}); {KEPT}",
                        hosted.coordinator,
                        hosted.describe()
                    ),
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
            .port
            .run_retained_inspect(&record.environment_id, &record.retained_inspect_argv)
            .await
        {
            Ok(metadata) => self.registry.record_inspect_success(claim, metadata),
            Err(e) => self.registry.record_inspect_failure(claim, &e),
        }
    }
}

/// Wire spelling of a run status for operator-facing reasons.
fn status_name(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Setup => "setup",
        RunStatus::Running => "running",
        RunStatus::Paused => "paused",
        RunStatus::Succeeded => "succeeded",
        RunStatus::Blocked => "blocked",
        RunStatus::Failed => "failed",
        RunStatus::Cancelled => "cancelled",
        RunStatus::BudgetExhausted => "budget-exhausted",
    }
}
