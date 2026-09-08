use super::*;

/// Apply an observed lifecycle transition only to the work turn it governs.
pub(in crate::interface::cli) fn suspend_swarm_turn(
    handle: &CancelHandle,
    generation: u64,
    still_current: impl FnOnce() -> bool,
) {
    let mut slot = handle.lock().unwrap();
    if matches!(*slot, CancelSlot::ScopedArmed(_, crate::domain::swarm::RunStatus::Setup | crate::domain::swarm::RunStatus::Running, admitted) if admitted <= generation)
        && still_current()
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
        let mut slot = handle.lock().unwrap();
        *slot = match std::mem::replace(&mut *slot, CancelSlot::Idle) {
            CancelSlot::Armed(sender) => CancelSlot::ScopedArmed(sender, status, generation),
            other => other,
        };
    }
    Some(receiver)
}

impl TurnControl {
    pub fn queue_swarm_wake(&self, generation: u64) -> bool {
        let mut pending = self.pending_swarm_wake.lock().unwrap();
        let first = pending.is_none();
        *pending = Some(pending.unwrap_or(0).max(generation));
        first
    }
    pub fn take_swarm_wake(&self, fallback: u64) -> u64 {
        self.pending_swarm_wake
            .lock()
            .unwrap()
            .take()
            .unwrap_or(fallback)
    }
}

#[cfg(test)]
#[path = "uds_swarm_turn_tests.rs"]
mod swarm_turn_scope_tests;
