//! Swarm lifecycle capability: sequencing and fail-closed ownership
//! decisions over the effect ports in [`ports`].
use crate::domain::error::DomainError;
use crate::domain::swarm::{Member, MemberExit, MemberStatus, Snapshot};

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

/// Whether `member`'s harness can no longer act: its row is dead, it has no
/// row, or its harness was observed gone.
fn gone(
    snapshot: &Snapshot,
    member: &str,
    observation: &(impl ProcessObservation + ?Sized),
) -> bool {
    match snapshot.members.iter().find(|m| m.id == member) {
        None => true,
        Some(row) => {
            row.status == MemberStatus::Dead
                || row
                    .process
                    .as_ref()
                    .is_some_and(|p| observation.harness_dead(p))
        }
    }
}

/// Whether `actor` ends `member` when the run settles (#2121). A member is
/// ended by the harness that launched it, which records the end as
/// deliberate before the process exits, so no "exited unexpectedly" note is
/// posted. The coordinator also ends members nobody else will: launched by
/// no member, or by one that is gone. Once the coordinator is gone, any
/// member ends the rest so the run is never stranded.
fn ends(
    snapshot: &Snapshot,
    actor: &str,
    member: &Member,
    coordinator_gone: bool,
    observation: &(impl ProcessObservation + ?Sized),
) -> bool {
    let coordinating = actor == snapshot.coordinator;
    match member.launcher.as_deref() {
        _ if coordinator_gone => true,
        Some(launcher) if launcher == actor => true,
        Some(launcher) => coordinating && gone(snapshot, launcher, observation),
        None => coordinating,
    }
}

pub async fn settle(
    snapshot: &Snapshot,
    actor: &str,
    processes: &(impl ProcessControl + ?Sized),
    observation: &(impl ProcessObservation + ?Sized),
) -> Result<(), DomainError> {
    debug_assert!(
        snapshot
            .members
            .iter()
            .filter(|m| m.id == snapshot.coordinator)
            .count()
            <= 1,
        "a run has one coordinator row"
    );
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
    let coordinating = actor == snapshot.coordinator;
    if coordinating && snapshot.status.abort_coordinator() {
        processes.suspend_local_inference(snapshot);
    }
    let coordinator_gone = gone(snapshot, &snapshot.coordinator, observation);
    // A member stops its own work and waits for its launcher to end it. The
    // coordinator's harness stays for reporting; a member of a
    // coordinator-less run is aborted with the rest below.
    if let (false, false) = (coordinating, coordinator_gone) {
        processes.suspend_local_inference(snapshot);
    }
    let mut members: Vec<_> = snapshot
        .members
        .iter()
        .filter(|m| m.status == MemberStatus::Live && m.id != snapshot.coordinator)
        .filter(|m| ends(snapshot, actor, m, coordinator_gone, observation))
        .collect();
    members.sort_by_key(|m| m.id == actor);
    end_members(&members, processes).await
}

/// A member still alive well after its run settled (#2121): its launcher
/// could not end it, so it ends itself rather than keep the environment
/// alive. Its launcher then reports the exit, which is the honest signal for
/// a teardown that did not go as planned. The coordinator never ends itself.
pub async fn settle_overdue(
    snapshot: &Snapshot,
    actor: &str,
    processes: &(impl ProcessControl + ?Sized),
) -> Result<(), DomainError> {
    let stranded: Vec<_> = snapshot
        .members
        .iter()
        .filter(|m| {
            snapshot.status.terminal()
                && m.id == actor
                && m.id != snapshot.coordinator
                && m.status == MemberStatus::Live
        })
        .collect();
    end_members(&stranded, processes).await
}

/// Abort and terminate each member in order; one unreachable member never
/// leaves the rest running.
async fn end_members(
    members: &[&Member],
    processes: &(impl ProcessControl + ?Sized),
) -> Result<(), DomainError> {
    let mut failures = Vec::new();
    for member in members {
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
        observation: &'a (dyn ProcessObservation + Sync),
    ) -> PortFuture<'a, Result<(), DomainError>> {
        Box::pin(settle(snapshot, actor, processes, observation))
    }
    fn settle_overdue<'a>(
        &'a self,
        snapshot: &'a Snapshot,
        actor: &'a str,
        processes: &'a dyn ProcessControl,
    ) -> PortFuture<'a, Result<(), DomainError>> {
        Box::pin(settle_overdue(snapshot, actor, processes))
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
