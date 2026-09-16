//! Swarm lifecycle capability: sequencing and fail-closed ownership
//! decisions over the effect ports in [`ports`].
use crate::domain::error::DomainError;
use crate::domain::swarm::{MemberExit, MemberStatus, Snapshot};

pub mod ports;

use ports::{
    Clock, CoordinationPort, PortFuture, ProcessControl, ProcessObservation, SwarmLifecycle,
};

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
/// authoritative for the member's harness, so the member is confirmed dead
/// (its active tasks block for `recover`) before the ordinary reconcile
/// runs, which then finds nothing to quarantine for it. `exit` says whether
/// the member's own teardown ran (its file reservations are released) or it
/// ended abruptly (they are retained for the coordinator to free). A socket
/// loss never reaches here; only the reaper's process-exit observation does.
/// A member already dead (or unknown to the store) is left as it is.
pub fn member_exited(
    coordination: &(impl CoordinationPort + ?Sized),
    processes: &(impl ProcessObservation + ?Sized),
    member: &str,
    exit: MemberExit,
) -> Result<Snapshot, DomainError> {
    let snapshot = coordination.snapshot()?;
    if snapshot
        .members
        .iter()
        .any(|m| m.id == member && m.status != MemberStatus::Dead)
    {
        coordination.confirm_dead(member, exit)?;
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
        exit: MemberExit,
    ) -> Result<Snapshot, DomainError> {
        member_exited(coordination, processes, member, exit)
    }
    fn settle<'a>(
        &'a self,
        snapshot: &'a Snapshot,
        actor: &'a str,
        processes: &'a dyn ProcessControl,
    ) -> PortFuture<'a, Result<(), DomainError>> {
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
