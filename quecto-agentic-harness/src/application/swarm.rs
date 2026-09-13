//! Lifecycle sequencing and fail-closed ownership decisions over effect ports.
use crate::domain::error::DomainError;
use crate::domain::subagent_launch::LaunchFuture;
use crate::domain::swarm::ProcessIdentity;
use crate::domain::swarm::{CoordinationPort, Member, MemberStatus, RunStatus, Snapshot};

// ── Capability-local ports (moved out of the domain by #1940) ────────────

pub trait ProcessObservation {
    fn harness_dead(&self, process: &ProcessIdentity) -> bool;
}

pub trait ProcessControl: Sync {
    /// Cancel this member's detached execution registry independently of turn abort.
    fn cancel_local_executions(&self);
    /// Cancel current jobs while retaining admission for a later resume.
    fn suspend_local_executions(&self, snapshot: &Snapshot);
    /// Suspend only this process; never signal a future turn or another member.
    fn suspend_local_inference(&self, snapshot: &Snapshot);
    fn abort<'a>(&'a self, member: &'a Member) -> LaunchFuture<'a, bool>;
    /// End the member's harness by delegation (#1939): the shutdown protocol
    /// over the endpoint it registered, the locally owned handle only when
    /// this harness launched it. The member's `ProcessIdentity` is an
    /// observation for liveness, never an authority to signal; a member
    /// reachable neither way is reported failed, not signalled.
    fn terminate<'a>(&'a self, member: &'a Member) -> LaunchFuture<'a, Result<(), DomainError>>;
}

pub trait Clock {
    fn now_seconds(&self) -> f64;
}

/// Application lifecycle entrypoint, injected by the composition root.
pub trait SwarmLifecycle: std::fmt::Debug + Send + Sync {
    fn reconcile(
        &self,
        coordination: &dyn CoordinationPort,
        processes: &dyn ProcessObservation,
    ) -> Result<Snapshot, DomainError>;
    /// An authoritative exit of a member this harness launched (#1961).
    fn member_exited(
        &self,
        coordination: &dyn CoordinationPort,
        processes: &dyn ProcessObservation,
        member: &str,
    ) -> Result<Snapshot, DomainError>;
    fn settle<'a>(
        &'a self,
        snapshot: &'a Snapshot,
        actor: &'a str,
        processes: &'a dyn ProcessControl,
    ) -> LaunchFuture<'a, Result<(), DomainError>>;
    fn observed_outcome(&self, snapshot: &Snapshot, clock: &dyn Clock) -> RunStatus;
}

pub fn reconcile(
    coordination: &(impl CoordinationPort + ?Sized),
    processes: &(impl ProcessObservation + ?Sized),
) -> Result<Snapshot, DomainError> {
    let snapshot = coordination.snapshot()?;
    for member in &snapshot.members {
        if member.status != MemberStatus::Dead
            && member
                .process
                .as_ref()
                .is_some_and(|p| processes.harness_dead(p))
        {
            // A vanished harness leaves independently grouped tool descendants
            // unaccounted for. Never release their files or admit replacements.
            coordination.quarantine(&member.id)?;
        }
    }
    coordination.snapshot()
}

/// The launching harness reaped `member`'s owned process (#1961): the exit is
/// authoritative for the member's whole process group, so the member is
/// confirmed dead (its active tasks block for `recover`) before the ordinary
/// reconcile runs, which then finds nothing to quarantine for it. A socket
/// loss never reaches here; only the reaper's process-exit observation does.
/// A member already dead (or unknown to the store) is left as it is.
pub fn member_exited(
    coordination: &(impl CoordinationPort + ?Sized),
    processes: &(impl ProcessObservation + ?Sized),
    member: &str,
) -> Result<Snapshot, DomainError> {
    let snapshot = coordination.snapshot()?;
    if snapshot
        .members
        .iter()
        .any(|m| m.id == member && m.status != MemberStatus::Dead)
    {
        coordination.confirm_dead(member)?;
    }
    reconcile(coordination, processes)
}

pub async fn settle(
    snapshot: &Snapshot,
    actor: &str,
    processes: &(impl ProcessControl + ?Sized),
) -> Result<(), DomainError> {
    if snapshot.status == crate::domain::swarm::RunStatus::Paused {
        // Every pause suspends local executions admitted up to this control
        // generation and keeps the registry open for a resume. An ended run
        // (#1729) also keeps its coordinator's turn alive so it can report
        // the outcome it proposed; every other member's inference suspends.
        processes.suspend_local_executions(snapshot);
        if !(snapshot.ended() && actor == snapshot.coordinator) {
            processes.suspend_local_inference(snapshot);
        }
        return Ok(());
    }
    if !snapshot.status.terminal() {
        return Ok(());
    }
    processes.cancel_local_executions();
    if actor == snapshot.coordinator && snapshot.status.abort_coordinator() {
        processes.suspend_local_inference(snapshot);
    }
    let mut members: Vec<_> = snapshot.members.iter().collect();
    members.sort_by_key(|m| m.id == actor);
    // Every live member is asked; a member that cannot be ended is reported
    // after the others were still asked, so one unreachable member never
    // leaves the rest running.
    let mut failures = Vec::new();
    for member in members {
        if member.status == MemberStatus::Dead {
            continue;
        }
        let coordinator = member.id == snapshot.coordinator;
        if coordinator {
            continue;
        }
        processes.abort(member).await;
        if let Err(error) = processes.terminate(member).await {
            failures.push(error.to_string());
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(DomainError::Tool(format!(
            "swarm settlement could not end every member: {}",
            failures.join("; ")
        )))
    }
}

pub fn observed_outcome(
    snapshot: &Snapshot,
    clock: &(impl Clock + ?Sized),
) -> crate::domain::swarm::RunStatus {
    use crate::domain::swarm::RunStatus;
    // A passed deadline ends the run as a resumable pause holding
    // `budget-exhausted` (#1729); the store records it on its next operation.
    if snapshot.status == RunStatus::Running && clock.now_seconds() >= snapshot.deadline {
        RunStatus::Paused
    } else {
        snapshot.status
    }
}

pub fn settlement_due(snapshot: &Snapshot, clock: &(impl Clock + ?Sized)) -> bool {
    observed_outcome(snapshot, clock).terminal()
}

#[derive(Debug)]
pub struct LifecycleService;
impl SwarmLifecycle for LifecycleService {
    fn reconcile(
        &self,
        coordination: &dyn CoordinationPort,
        processes: &dyn ProcessObservation,
    ) -> Result<Snapshot, DomainError> {
        reconcile(coordination, processes)
    }
    fn member_exited(
        &self,
        coordination: &dyn CoordinationPort,
        processes: &dyn ProcessObservation,
        member: &str,
    ) -> Result<Snapshot, DomainError> {
        member_exited(coordination, processes, member)
    }
    fn settle<'a>(
        &'a self,
        snapshot: &'a Snapshot,
        actor: &'a str,
        processes: &'a dyn ProcessControl,
    ) -> crate::domain::subagent_launch::LaunchFuture<'a, Result<(), DomainError>> {
        Box::pin(settle(snapshot, actor, processes))
    }
    fn observed_outcome(
        &self,
        snapshot: &Snapshot,
        clock: &dyn Clock,
    ) -> crate::domain::swarm::RunStatus {
        observed_outcome(snapshot, clock)
    }
}

#[cfg(test)]
#[path = "swarm_tests.rs"]
mod tests;
