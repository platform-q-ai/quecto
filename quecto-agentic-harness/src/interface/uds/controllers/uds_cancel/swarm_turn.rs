use super::*;

/// Apply an observed lifecycle transition only to the work turn it governs.
// `still_current` is a trait object on purpose: one instantiation serves every
// caller instead of one monomorphized copy per closure.
pub(in crate::interface) fn suspend_swarm_turn(
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

pub(in crate::interface) async fn arm_swarm_cancel(
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
    /// A wake from outside this process (or a resume): it rides any
    /// pending message, and the message is no longer a self-wake alone.
    pub fn queue_swarm_wake(&self, generation: u64) -> bool {
        let mut pending = self
            .pending_swarm_wake
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let first = pending.is_none();
        let highest = pending.map_or(generation, |(seen, _)| seen.max(generation));
        *pending = Some((highest, None));
        first
    }
    /// A self-wake for suspension `epoch` (#2626 review): it sends its own
    /// message only when none is pending, and then carries the epoch.
    fn queue_self_wake(&self, generation: u64, epoch: u64) -> bool {
        let mut pending = self
            .pending_swarm_wake
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let first = pending.is_none();
        *pending = Some(match *pending {
            None => (generation, Some(epoch)),
            Some((seen, source)) => (seen.max(generation), source),
        });
        first
    }
    /// Fold `generation` into a pending wake message, if one is pending:
    /// returns whether it did. An empty slot is left empty, so a wake the
    /// full channel cannot carry never occupies (or later clears) the slot
    /// (#2626 review round 5). The joiner is an outside or resume wake, so
    /// the message is no longer a self-wake alone: its tag is cleared, as
    /// `queue_swarm_wake` clears it (#2626 review round 6).
    pub fn join_pending_swarm_wake(&self, generation: u64) -> bool {
        let mut pending = self
            .pending_swarm_wake
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match *pending {
            Some((seen, _)) => {
                *pending = Some((seen.max(generation), None));
                true
            }
            None => false,
        }
    }
    /// Release the pending wake slot a wake could not be sent for (a full
    /// or closed channel), keeping any deferred generation for the next
    /// wake (#2626 review round 4): only the slot is cleared.
    pub fn release_swarm_wake(&self) {
        self.pending_swarm_wake
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
    }
    pub fn take_swarm_wake(&self, fallback: u64) -> u64 {
        self.take_swarm_wake_from(fallback).0
    }
    /// The wake to apply, and the suspension epoch when only a self-wake
    /// sent it.
    pub fn take_swarm_wake_from(&self, fallback: u64) -> (u64, Option<u64>) {
        let deferred = self
            .deferred_swarm_wake
            .swap(0, std::sync::atomic::Ordering::SeqCst);
        let (generation, self_epoch) = self
            .pending_swarm_wake
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .unwrap_or((fallback, None));
        (generation.max(deferred), self_epoch)
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
        waker: tokio::sync::mpsc::WeakSender<
            crate::interface::uds::framing::uds_multi::ClientMessage,
        >,
    ) {
        let _ = self.self_waker.set(waker);
    }

    /// Wake this process after `delay` (#2531) at the latest control
    /// generation it has seen, for suspension `epoch` (a newer suspension
    /// makes it stale): a wake at or below the run's wake cursor claims
    /// nothing, but a suspended member's probe still runs. One is
    /// scheduled at a time; returns whether this call scheduled it. Without
    /// a bound channel, or once the loop or the process is ending, nothing
    /// is sent.
    pub(crate) fn schedule_self_wake(
        self: &Arc<Self>,
        delay: std::time::Duration,
        epoch: u64,
    ) -> bool {
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
            if control.queue_self_wake(generation, epoch) {
                let message = crate::interface::uds::framing::uds_multi::ClientMessage::SwarmWake {
                    generation,
                };
                if sender.send(message).await.is_err() {
                    // The loop ended while sending: release the slot.
                    control.release_swarm_wake();
                }
            }
        });
        true
    }
}

#[cfg(test)]
mod swarm_turn_scope_tests;
