//! Pure environment retention policy (#1924, #1939).
//!
//! What the domain decides about an emptied environment: whether its
//! final-member teardown is withheld and the operator-facing reason. The
//! orchestration around these decisions — observing the hosted swarm run,
//! recording a coordinator loss, running the retained scripts — belongs to
//! the `environments` application capability and its ports; nothing here
//! performs an effect.

use crate::domain::swarm::RunStatus;

/// Why a member is being finalized. Normal exits stop the emptied
/// environment with the retained `kill`; a launch rollback (the launch
/// failed after the environment was created) runs the retained `cleanup`
/// instead, per the documented `cleanup` contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberFinalizeMode {
    /// The member ended on its own: a post-mortem (inspect runs).
    Exit,
    /// Parent-initiated termination that may not be the owner's word: a
    /// harness shutdown (which can be a crash) or the member side of an
    /// environment kill. Death by our own hand is not a post-mortem.
    ParentKill,
    /// The owner ended this member on purpose (#2206): its own selected
    /// termination of the one member (`agent_cmd kill`, a kill from the
    /// owning TUI, one forwarded down the owner's chain), or a one-shot
    /// parent ending the run it was started for. Toward a swarm it is
    /// exactly a `ParentKill`: a run its owner has not closed keeps its
    /// box. A plain container child has nothing to resume, so its
    /// environment is cleaned up and its record forgotten.
    OwnerEnd,
    /// The owner explicitly ended everything it owns (#2070): delete-all, or
    /// a session transition that is going to succeed. Its swarms end with it,
    /// so nothing is kept — whatever state their runs are in. A harness that
    /// is shutting down is this only when its owner announced the exit
    /// first; unannounced it stays a `ParentKill`, because that can be a
    /// crash (a signal, its last client gone, its parent lost).
    OwnerTeardown,
    /// Rollback of a failed join into an environment someone else created.
    LaunchRollback,
    /// Rollback of the launch that created the environment: the environment
    /// never became usable, so its record is discarded after the cleanup.
    LaunchRollbackOwned,
}

impl MemberFinalizeMode {
    /// Whether the mode may withhold the teardown for inspection (#1924):
    /// only an end that leaves something worth inspecting.
    pub const fn inspectable_end(self) -> bool {
        matches!(self, Self::Exit | Self::ParentKill | Self::OwnerEnd)
    }

    /// Whether the owner ended this one member on purpose (#2206). An
    /// owner's explicit teardown of everything (#2070) is not this: it
    /// ends every box with the retained kill without reading any store.
    pub const fn owner_ended_member(self) -> bool {
        matches!(self, Self::OwnerEnd)
    }

    /// Whether the mode is a launch rollback (retained `cleanup`, never
    /// `kill`).
    pub const fn launch_rollback(self) -> bool {
        matches!(self, Self::LaunchRollback | Self::LaunchRollbackOwned)
    }
}

/// The swarm run an environment hosts, as observed from outside it (#1924).
#[derive(Debug, Clone, PartialEq)]
pub struct HostedSwarmRun {
    /// The run's own id, the name an operator sees in `swarm_control`
    /// receipts and the reason a box is kept for an unfinished run.
    pub id: String,
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

    /// The run's owner closed it (#2070): the supervisor outside the swarm
    /// made a held outcome terminal. `cancelled` is NOT this — the
    /// coordinator agent writes it itself, and an agent never ends a swarm.
    /// This trusts the store's own rule that only the supervisor closes a
    /// run; an agent with a shell inside the box can already destroy its
    /// checkout, so that rule is a convention, not a security boundary.
    pub fn closed_by_owner(&self) -> bool {
        self.status.proposable()
    }

    /// Whether an emptied environment hosting this run is kept: a created
    /// run its owner has not closed can still be resumed.
    pub fn keeps_environment(&self) -> bool {
        self.created() && !self.closed_by_owner()
    }

    /// The bootstrap placeholder every container carries (#1715): no
    /// swarm was ever created in it.
    pub fn placeholder(&self) -> bool {
        !self.created()
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
    /// No board the host could look at (#2206 round 3): the record
    /// advertises no checkout and its workspace is absent from the host (a
    /// third-party script set whose workspace lives only in the container).
    /// Nothing says it hosts no swarm, so it is never a plain container —
    /// but it is no store the host could read either, so the retained kill
    /// runs as it always has, except on the owner's end of the member,
    /// which keeps it (for `container kill` or `gc`).
    NoStoreUnverified,
    /// The store's run.
    Run(HostedSwarmRun),
    /// A store exists but could not be read (contended, corrupt, no
    /// interpreter). A store exists only where members coordinate, so this
    /// is not proof that no run is live.
    Unreadable(String),
}

impl SwarmRunObservation {
    /// A plain container (#2206): it advertises no coordination store, or
    /// its store holds only the bootstrap placeholder. A created swarm run,
    /// whatever its state, and a store that cannot be read are not.
    pub fn plain_container(&self) -> bool {
        match self {
            Self::NoStore => true,
            Self::Run(hosted) => hosted.placeholder(),
            Self::NoStoreUnverified | Self::Unreadable(_) => false,
        }
    }
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
/// (#1924, #2070): a member whose exit empties the environment while it
/// hosts a created swarm run is its coordinator (workers are the
/// coordinator's in-container descendants, never members of the supervising
/// session's environment record). A swarm's container lives as long as the
/// swarm and no longer. A swarm ends only when its owner says so — the
/// supervisor closing the run into its outcome, or the owner explicitly
/// tearing down everything it owns — and then the box, its checkout and its
/// board go with it. A crash, a lost coordinator, an agent that exited or a
/// coordinator that cancelled its own run does NOT end the swarm: the box is
/// kept so the run can be resumed, for the member's own exit, for a
/// supervisor's `kill` of that one member and for a harness shutdown (which
/// may be a crash) alike. A launch rollback
/// (nothing to keep) keeps its cleanup. A store that exists but cannot be
/// read is kept too: it is never proof the run ended, and an explicit kill
/// can always remove it later, while a destroyed box cannot be recovered.
pub fn retains_environment(mode: MemberFinalizeMode, observed: &SwarmRunObservation) -> bool {
    mode.inspectable_end()
        && match observed {
            SwarmRunObservation::NoStore => false,
            // #2206 round 3: never ended for good on the owner's word —
            // kept instead; every other end runs the retained kill as
            // before.
            SwarmRunObservation::NoStoreUnverified => mode.owner_ended_member(),
            SwarmRunObservation::Run(hosted) => hosted.keeps_environment(),
            SwarmRunObservation::Unreadable(_) => true,
        }
}

/// Whether an emptied environment is ended for good (#2206): its retained
/// `cleanup` runs (the container and its state directory go) and, once
/// that succeeded, its record is forgotten, as a launch rollback does. Only
/// on the owner's word, and only for a plain container: it hosts no swarm,
/// so there is nothing to resume or inspect. A swarm — whatever its run's
/// state — and a store that cannot be read are never ended here; the swarm
/// rules above decide them.
pub fn ends_plain_environment(mode: MemberFinalizeMode, observed: &SwarmRunObservation) -> bool {
    mode.owner_ended_member() && observed.plain_container()
}

const KEPT: &str = "environment retained for inspection, kill_container to remove";

/// The `metadata.retained` reason for a withheld teardown (#1924).
pub fn retention_reason(mode: MemberFinalizeMode, observed: &SwarmRunObservation) -> String {
    match observed {
        SwarmRunObservation::Unreadable(error) => format!(
            "final member gone while the coordination store could not be read ({error}); {KEPT}"
        ),
        SwarmRunObservation::NoStore => format!("final member gone; {KEPT}"),
        SwarmRunObservation::NoStoreUnverified => format!(
            "final member ended by its owner, but its workspace is not on this host, so whether it hosts a swarm cannot be checked; {KEPT}"
        ),
        SwarmRunObservation::Run(run)
            if matches!(
                mode,
                MemberFinalizeMode::ParentKill | MemberFinalizeMode::OwnerEnd
            ) =>
        {
            format!(
                "coordinator killed by supervisor; run {}; {KEPT}",
                run.describe()
            )
        }
        SwarmRunObservation::Run(run) => match (run.status, run.outcome) {
            (RunStatus::Cancelled, _) => format!("run ended: cancelled; {KEPT}"),
            (RunStatus::Paused, Some(outcome)) => {
                format!("run ended: {}; {KEPT}", status_name(outcome))
            }
            _ => format!("run {}; {KEPT}", run.describe()),
        },
    }
}

/// The reason once a coordinator loss was recorded: a real loss names the
/// lost coordinator; a run that had already ended keeps the ordinary
/// retention reason.
pub fn loss_reason(coordinator: &str, loss: &CoordinatorLoss) -> String {
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

/// The reason when the loss could not be recorded on the store: the
/// environment is still retained (losing the box is never the safer
/// outcome), and the reason says why the run's own record is stale.
pub fn unrecorded_loss_reason(hosted: &HostedSwarmRun, error: &str) -> String {
    format!(
        "swarm coordinator '{}' lost its connection while the run was {}; the loss could not be recorded ({error}); {KEPT}",
        hosted.coordinator,
        hosted.describe()
    )
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
