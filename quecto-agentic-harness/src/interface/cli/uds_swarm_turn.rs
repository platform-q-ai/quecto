use super::*;

/// Apply an observed lifecycle transition only to the work turn it governs.
// `still_current` is a trait object on purpose: one instantiation serves every
// caller instead of one monomorphized copy per closure.
pub(in crate::interface::cli) fn suspend_swarm_turn(
    handle: &CancelHandle,
    generation: u64,
    still_current: &dyn Fn() -> bool,
) {
    use crate::domain::swarm::value_objects::lifecycle::RunStatus;
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
    // A shutdown is executing (#1936): the turn is refused before the slot
    // is touched, exactly like a pre-fired cancel, so the loop returns to
    // its exit signal instead of starting a provider call it would then
    // have to be signalled out of.
    if control.is_shutting_down() {
        return None;
    }
    let scope = if let Some(port) = &control.swarm_control {
        match port
            .apply(crate::domain::swarm::value_objects::run_control::RunControlAction::Status)
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
        let deferred = self
            .deferred_swarm_wake
            .swap(0, std::sync::atomic::Ordering::SeqCst);
        self.pending_swarm_wake
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .unwrap_or(fallback)
            .max(deferred)
    }

    /// Dispatch loop: keep a wake it could not apply (store contention) for
    /// the next wake the reader delivers. The coalescing slot stays free so
    /// that reader wake still sends its message; `take_swarm_wake` folds the
    /// deferred generation in (#1721).
    pub fn defer_swarm_wake(&self, generation: u64) {
        self.deferred_swarm_wake
            .fetch_max(generation, std::sync::atomic::Ordering::SeqCst);
    }

    /// Bind the dispatch loop's command channel for self-wakes (#2531),
    /// weakly so it never keeps the loop alive; the first binding stands.
    pub(crate) fn bind_self_waker(
        &self,
        waker: tokio::sync::mpsc::WeakSender<crate::interface::cli::uds_multi::ClientMessage>,
    ) {
        let _ = self.self_waker.set(waker);
    }

    /// Wake this process after `delay` (#2531) at the latest control
    /// generation it has seen: a wake at or below the run's wake cursor
    /// claims nothing, but a suspended member's probe still runs. One is
    /// scheduled at a time; returns whether this call scheduled it. Without
    /// a bound channel, or once the loop or the process is ending, nothing
    /// is sent.
    pub(crate) fn schedule_self_wake(self: &Arc<Self>, delay: std::time::Duration) -> bool {
        let Some(waker) = self.self_waker.get().cloned() else {
            return false;
        };
        let scheduled = self.self_wake_scheduled.compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
        );
        match scheduled {
            Ok(_) => {}
            Err(_) => return false,
        }
        let control = Arc::clone(self);
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            control
                .self_wake_scheduled
                .store(false, std::sync::atomic::Ordering::SeqCst);
            let live = match control.is_shutting_down() {
                false => waker.upgrade(),
                true => None,
            };
            let Some(sender) = live else {
                return;
            };
            let generation = control.control_generation().unwrap_or(0);
            if control.queue_swarm_wake(generation) {
                let message =
                    crate::interface::cli::uds_multi::ClientMessage::SwarmWake { generation };
                if sender.send(message).await.is_err() {
                    // The loop ended while sending: release the slot.
                    control.take_swarm_wake(0);
                }
            }
        });
        true
    }
}

#[cfg(test)]
#[path = "uds_swarm_turn_tests.rs"]
mod swarm_turn_scope_tests;
