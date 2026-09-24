//! Swarm lifecycle capability: sequencing and fail-closed ownership
//! decisions over the effect ports in [`ports`].
use crate::domain::error::DomainError;
use crate::domain::swarm::{Member, MemberExit, MemberStatus, Snapshot};

pub mod ports;

use ports::{
    Clock, CoordinationPort, PortFuture, ProcessControl, ProcessObservation, SettlementStep,
    SwarmLifecycle,
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

/// How the settling harness takes part in ending a terminal run (#2121).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SettlingAs {
    /// Ends the members it launched and those whose launcher is gone; its
    /// own harness stays for reporting.
    Coordinator,
    /// Ends only the members it launched, then waits to be ended.
    Member,
    /// A member of a run whose coordinator is gone: ends its launchees and
    /// the members nobody else will end, itself last.
    Orphan,
}

/// Whether `actor` ends `member` when the run settles. A member is ended by
/// the harness that launched it, which records the end as deliberate before
/// the process exits, so no "exited unexpectedly" note is posted. Members
/// whose launcher is gone are ended by the coordinator, or by every member
/// once the coordinator is gone too, so the run is never stranded.
fn ends(
    snapshot: &Snapshot,
    actor: &str,
    member: &Member,
    role: SettlingAs,
    observation: &(impl ProcessObservation + ?Sized),
) -> bool {
    match (member.launcher.as_deref(), role) {
        (Some(launcher), _) if launcher == actor => true,
        (_, SettlingAs::Orphan) if member.id == actor => true,
        (Some(launcher), SettlingAs::Coordinator | SettlingAs::Orphan) => {
            gone(snapshot, launcher, observation)
        }
        (None, SettlingAs::Coordinator | SettlingAs::Orphan) => true,
        _ => false,
    }
}

/// The next step for `actor` `elapsed` after it first settled (#2121).
/// `grace` is how long the harness's own teardown ladder may take to end a
/// member, so a member only ends itself once its launcher had time to.
pub fn settlement_step(
    snapshot: &Snapshot,
    actor: &str,
    elapsed: std::time::Duration,
    grace: std::time::Duration,
) -> SettlementStep {
    let awaiting_end = snapshot.status.terminal()
        && actor != snapshot.coordinator
        && snapshot
            .members
            .iter()
            .any(|m| m.id == actor && m.status == MemberStatus::Live);
    match (awaiting_end, elapsed >= grace) {
        (true, true) => SettlementStep::EndSelf,
        (true, false) => SettlementStep::Wait,
        (false, _) => SettlementStep::Done,
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
    let role = match (
        coordinating,
        gone(snapshot, &snapshot.coordinator, observation),
    ) {
        (true, _) => SettlingAs::Coordinator,
        (false, true) => SettlingAs::Orphan,
        (false, false) => SettlingAs::Member,
    };
    if role == SettlingAs::Member {
        // It stops its own work and waits for its launcher to end it.
        processes.suspend_local_inference(snapshot);
    }
    let (itself, others): (Vec<_>, Vec<_>) = snapshot
        .members
        .iter()
        .filter(|m| m.status == MemberStatus::Live && m.id != snapshot.coordinator)
        .filter(|m| ends(snapshot, actor, m, role, observation))
        .partition(|m| m.id == actor);
    debug_assert!(
        itself.is_empty() || role == SettlingAs::Orphan,
        "only a member of a coordinator-less run ends itself at settlement"
    );
    // The others end at once, not one after another, so no member outlives
    // its grace waiting in a queue; the actor ends last.
    let others = end_members(&others, processes).await;
    let itself = end_members(&itself, processes).await;
    others.and(itself)
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
    let settled_member = snapshot.status.terminal() && actor != snapshot.coordinator;
    let stranded: Vec<_> = snapshot
        .members
        .iter()
        .filter(|m| settled_member && m.id == actor && m.status == MemberStatus::Live)
        .collect();
    end_members(&stranded, processes).await
}

/// Abort and terminate the members concurrently; one unreachable or slow
/// member never holds up or leaves the rest running.
async fn end_members(
    members: &[&Member],
    processes: &(impl ProcessControl + ?Sized),
) -> Result<(), DomainError> {
    let outcomes = futures::future::join_all(members.iter().map(|member| async move {
        processes.abort(member).await;
        processes.terminate(member).await
    }))
    .await;
    let failures: Vec<_> = outcomes
        .into_iter()
        .filter_map(Result::err)
        .map(|error| error.to_string())
        .collect();
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
    fn settlement_step(
        &self,
        snapshot: &Snapshot,
        actor: &str,
        elapsed: std::time::Duration,
        grace: std::time::Duration,
    ) -> SettlementStep {
        settlement_step(snapshot, actor, elapsed, grace)
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
