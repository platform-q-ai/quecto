use super::*;

/// Apply an observed lifecycle transition only to the work turn it governs.
// `still_current` is a trait object on purpose: one instantiation serves every
// caller instead of one monomorphized copy per closure.
pub(in crate::interface::cli) fn suspend_swarm_turn(
    handle: &CancelHandle,
    generation: u64,
    still_current: &dyn Fn() -> bool,
) {
    use crate::domain::swarm::RunStatus;
    // Decide under the lock, verify with the lock released (the verification
    // may run a subprocess), then re-check the same armed generation before
    // acting so abort/steer are never blocked behind coordination I/O.
    let admitted = {
        let slot = handle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match &*slot {
            CancelSlot::ScopedArmed(_, RunStatus::Setup | RunStatus::Running, admitted)
                if *admitted <= generation =>
            {
                Some(*admitted)
            }
            _ => None,
        }
    };
    let Some(admitted) = admitted else {
        return;
    };
    if !still_current() {
        return;
    }
    let mut slot = handle
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if matches!(&*slot, CancelSlot::ScopedArmed(_, RunStatus::Setup | RunStatus::Running, current) if *current == admitted)
    {
        *slot = CancelSlot::Idle;
    }
}

pub(in crate::interface::cli) async fn arm_swarm_cancel(
    handle: &CancelHandle,
    control: &TurnControlHandle,
) -> Option<tokio::sync::oneshot::Receiver<()>> {
    let scope = if let Some(port) = &control.swarm_control {
        match port
            .apply(crate::domain::swarm::RunControlAction::Status)
            .await
        {
            Ok(receipt) => Some((receipt.status, receipt.generation)),
            Err(error) => {
                tracing::error!(%error, "swarm turn admission unavailable");
                return None;
            }
        }
    } else {
        None
    };
    let receiver = arm_cancel(handle)?;
    if let Some((status, generation)) = scope {
        let mut slot = handle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *slot = match std::mem::replace(&mut *slot, CancelSlot::Idle) {
            CancelSlot::Armed(sender) => CancelSlot::ScopedArmed(sender, status, generation),
            other => other,
        };
    }
    Some(receiver)
}

impl TurnControl {
    pub fn queue_swarm_wake(&self, generation: u64) -> bool {
        let mut pending = self
            .pending_swarm_wake
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let first = pending.is_none();
        *pending = Some(pending.unwrap_or(0).max(generation));
        first
    }
    pub fn take_swarm_wake(&self, fallback: u64) -> u64 {
        self.pending_swarm_wake
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .unwrap_or(fallback)
    }
}

#[cfg(test)]
#[path = "uds_swarm_turn_tests.rs"]
mod swarm_turn_scope_tests;
