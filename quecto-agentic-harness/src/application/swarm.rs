//! Lifecycle sequencing and fail-closed ownership decisions over effect ports.
use crate::domain::error::DomainError;
use crate::domain::swarm::{
    CoordinationPort, MemberStatus, ProcessControl, ProcessObservation, Snapshot,
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

pub async fn settle(
    snapshot: &Snapshot,
    actor: &str,
    processes: &(impl ProcessControl + ?Sized),
) -> Result<(), DomainError> {
    if !snapshot.status.terminal() {
        return Ok(());
    }
    processes.cancel_local_executions();
    let mut members: Vec<_> = snapshot.members.iter().collect();
    members.sort_by_key(|m| m.id == actor);
    for member in members {
        if member.status == MemberStatus::Dead {
            continue;
        }
        let coordinator = member.id == snapshot.coordinator;
        if coordinator && !snapshot.status.abort_coordinator() {
            continue;
        }
        let aborted = processes.abort(member).await;
        if coordinator && aborted {
            continue;
        }
        if let Some(process) = &member.process {
            processes.terminate(process).await?;
        }
    }
    Ok(())
}

pub fn observed_outcome(
    snapshot: &Snapshot,
    clock: &(impl crate::domain::swarm::Clock + ?Sized),
) -> crate::domain::swarm::RunStatus {
    use crate::domain::swarm::RunStatus;
    if snapshot.status == RunStatus::Running && clock.now_seconds() >= snapshot.deadline {
        RunStatus::BudgetExhausted
    } else {
        snapshot.status
    }
}

pub fn settlement_due(
    snapshot: &Snapshot,
    clock: &(impl crate::domain::swarm::Clock + ?Sized),
) -> bool {
    observed_outcome(snapshot, clock).terminal()
}

#[derive(Debug)]
pub struct LifecycleService;
impl crate::domain::swarm::SwarmLifecycle for LifecycleService {
    fn reconcile(
        &self,
        coordination: &dyn CoordinationPort,
        processes: &dyn ProcessObservation,
    ) -> Result<Snapshot, DomainError> {
        reconcile(coordination, processes)
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
        clock: &dyn crate::domain::swarm::Clock,
    ) -> crate::domain::swarm::RunStatus {
        observed_outcome(snapshot, clock)
    }
}

#[cfg(test)]
#[path = "swarm_tests.rs"]
mod tests;
