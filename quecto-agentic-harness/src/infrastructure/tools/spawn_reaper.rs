//! Reaper for locally spawned subagent children.
//!
//! Local launches (only) own an OS child process, held by the one
//! [`OwnedChildSupervisor`] (#1935); this task waits for the supervisor's
//! exit report, publishes it on the shared exit signal, and hands the
//! authoritative process-exit observation to the application's
//! `ObserveOwnedChildExit` (#1936), which claims and runs — or joins — the
//! row's exactly-once compensation. Script-managed children have no local
//! process and rely on the monitor's socket-EOF observation instead.

use std::sync::Arc;

use crate::application::subagents::dto::{ObserveOwnedChildExitRequest, ObservedExit};
use crate::application::subagents::ports::ExitObservation;
use crate::application::subagents::use_cases::ObserveOwnedChildExit;
use crate::domain::subagent_teardown::DelegatedAgentIdentity;
use crate::infrastructure::processes::owned_child_supervisor::{
    ChildExit, ChildHandleId, OwnedChildSupervisor,
};

use super::subagent_registry::{ExitSignal, ExitSignalTx};

pub struct ReaperContext {
    pub exit_tx: ExitSignalTx,
    pub child: DelegatedAgentIdentity,
    pub observer: Arc<ObserveOwnedChildExit>,
    /// The swarm this launch reserved a member in, with that member's id
    /// (#1961): the reaped exit is the authoritative death of the member.
    pub swarm_member: Option<(super::swarm_bridge::SwarmContext, String)>,
}

pub fn spawn_reaper_task(
    handle: ChildHandleId,
    supervisor: Arc<OwnedChildSupervisor>,
    context: ReaperContext,
) {
    let ReaperContext {
        exit_tx,
        child,
        observer,
        swarm_member,
    } = context;
    tokio::spawn(async move {
        let exit = supervisor.wait_exit(handle).await;
        let member_exit = member_exit_kind(exit.as_ref(), &supervisor.signals_sent(handle));
        // send_replace: store the real exit status even when no awaiter holds
        // a receiver yet, so late awaits report it instead of a fallback.
        exit_tx.send_replace(Some(exit_signal_from_exit(exit)));
        let observed = observer
            .execute(ObserveOwnedChildExitRequest {
                child: child.clone(),
                observation: ExitObservation::ProcessExited,
            })
            .await;
        match &observed {
            ObservedExit::Compensated { removed } => {
                tracing::info!(agent = %child.uuid, removed = removed.len(), "reaper: child compensated");
            }
            ObservedExit::Joined(observation) => {
                tracing::debug!(agent = %child.uuid, ?observation, "reaper: joined the child's compensation");
            }
            ObservedExit::DeferredToProcessExit => {
                debug_assert!(false, "a process exit is never deferred to itself");
            }
        }
        if let Some((context, member)) = swarm_member {
            let _ = tokio::task::spawn_blocking(move || {
                if let Err(error) =
                    super::swarm_lifecycle::member_exited(&context, &member, member_exit)
                {
                    tracing::error!(%error, member, "swarm reaper could not confirm the member's death; capacity retained");
                }
            })
            .await;
        }
        // The exit was published and every cleanup ran: the handle's slot
        // can go; retained registry clones then see "no retained handle".
        supervisor.retire(handle);
    });
}

/// How a swarm member's reaped exit reads to the store (#1961): an exit code
/// is the harness ending on its own terms (its teardown ran), a signal this
/// harness sent reached the member's whole process group, so both are
/// orderly; a signal nobody here sent (SIGKILL from the OOM killer, an
/// operator) or an unobservable end is abrupt — its Bash tool groups may
/// survive, so its reservations are retained.
pub fn member_exit_kind(
    exit: Option<&ChildExit>,
    sent: &[crate::infrastructure::processes::owned_child_supervisor::SentSignal],
) -> crate::domain::swarm::MemberExit {
    use crate::domain::swarm::MemberExit;
    match exit {
        Some(ChildExit::Code(_)) => MemberExit::Orderly,
        Some(ChildExit::Signal(_)) if !sent.is_empty() => MemberExit::Orderly,
        Some(ChildExit::Signal(_)) | Some(ChildExit::Unobservable(_)) | None => MemberExit::Abrupt,
    }
}

/// The supervisor's exit report in the registry's exit-signal vocabulary.
/// An unknown handle or an unobservable wait yields the empty signal, as a
/// failed `wait` always did.
pub(super) fn exit_signal_from_exit(exit: Option<ChildExit>) -> ExitSignal {
    match exit {
        Some(ChildExit::Code(code)) => ExitSignal {
            exit_code: Some(code),
            signal: None,
            kind: Default::default(),
        },
        Some(ChildExit::Signal(signal)) => ExitSignal {
            exit_code: None,
            signal: Some(signal),
            kind: Default::default(),
        },
        Some(ChildExit::Unobservable(_)) | None => ExitSignal {
            exit_code: None,
            signal: None,
            kind: Default::default(),
        },
    }
}

#[cfg(test)]
#[path = "spawn_reaper_tests.rs"]
mod tests;
